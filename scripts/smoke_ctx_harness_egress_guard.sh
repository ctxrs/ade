#!/usr/bin/env bash
set -euo pipefail

# Smoke test for the ctx-managed harness image's transparent egress guard pieces.
#
# This validates that:
# - iptables exists in the image
# - ctx-egress-proxy exists and can run
# - iptables OUTPUT redirect + proxy allowlist actually blocks/permits traffic for non-root
#
# Notes:
# - The proxy runs under a dedicated non-root bypass uid. Test traffic runs as a
#   separate non-root uid so it still exercises redirect + allowlist enforcement.

RUNTIME="${CONTAINER_RUNTIME:-nerdctl}"
IMAGE="${CTX_HARNESS_IMAGE:-ghcr.io/ctxrs/ctx-harness:ubuntu-24.04}"

ALLOW_HOST="${ALLOW_HOST:-example.com}"
BLOCK_HOST="${BLOCK_HOST:-google.com}"
PROXY_BYPASS_UID="${PROXY_BYPASS_UID:-43558}"

NAME="ctx-harness-egress-smoke-$$"
CFG_LOCAL="/tmp/${NAME}.egress-proxy.json"

cleanup() {
  rm -f "$CFG_LOCAL" >/dev/null 2>&1 || true
  "$RUNTIME" rm -f "$NAME" >/dev/null 2>&1 || true
}
trap cleanup EXIT

if ! command -v "$RUNTIME" >/dev/null 2>&1; then
  echo "missing container runtime: $RUNTIME" >&2
  exit 2
fi

"$RUNTIME" pull "$IMAGE" >/dev/null 2>&1 || true

"$RUNTIME" run -d --name "$NAME" --cap-add=NET_ADMIN "$IMAGE" >/dev/null

# Basic sanity checks.
"$RUNTIME" exec --user 0 "$NAME" sh -lc "command -v iptables >/dev/null 2>&1"
"$RUNTIME" exec --user 0 "$NAME" sh -lc "test -x /usr/local/bin/ctx-egress-proxy"
"$RUNTIME" exec --user 0 "$NAME" sh -lc "command -v curl >/dev/null 2>&1"

cat >"$CFG_LOCAL" <<EOF
{
  "listen": "127.0.0.1:15001",
  "mode": "allowlist",
  "allowlist": ["$ALLOW_HOST"],
  "bypass_uid": $PROXY_BYPASS_UID
}
EOF

"$RUNTIME" cp "$CFG_LOCAL" "$NAME:/tmp/ctx-egress-proxy.json"

# Start the proxy.
"$RUNTIME" exec --user 0 "$NAME" sh -lc '
  set -e
  RUST_LOG=info nohup /usr/local/bin/ctx-egress-proxy --config /tmp/ctx-egress-proxy.json >/tmp/ctx-egress-proxy.log 2>&1 &
  echo $! >/tmp/ctx-egress-proxy.pid
'

"$RUNTIME" exec --user 0 "$NAME" sh -lc '
  set -e
  pid="$(cat /tmp/ctx-egress-proxy.pid)"
  for _ in $(seq 1 50); do
    if ! kill -0 "$pid" 2>/dev/null; then
      cat /tmp/ctx-egress-proxy.log >&2 || true
      exit 1
    fi
    uid="$(awk "/^Uid:/{print \$2}" "/proc/$pid/status" 2>/dev/null || true)"
    if [ "$uid" = "'"$PROXY_BYPASS_UID"'" ]; then
      exit 0
    fi
    sleep 0.1
  done
  cat "/proc/$pid/status" >&2 || true
  cat /tmp/ctx-egress-proxy.log >&2 || true
  exit 1
'

# Configure iptables the same way ctx does (with a harmless daemon allow rule).
"$RUNTIME" exec --user 0 "$NAME" sh -lc '
  set -e
  daemon_ip="127.0.0.1"

  iptables -t nat -F OUTPUT || true
  iptables -F OUTPUT || true
  iptables -P OUTPUT DROP
  iptables -A OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
  iptables -A OUTPUT -d 127.0.0.1/8 -j ACCEPT
  iptables -A OUTPUT -o lo -j ACCEPT
  iptables -A OUTPUT -p udp --dport 53 -j ACCEPT
  iptables -A OUTPUT -p tcp --dport 53 -j ACCEPT
  iptables -A OUTPUT -d "$daemon_ip" -p tcp --dport 9 -j ACCEPT
  iptables -A OUTPUT -m owner --uid-owner '"$PROXY_BYPASS_UID"' -j ACCEPT

  iptables -t nat -A OUTPUT -m owner --uid-owner '"$PROXY_BYPASS_UID"' -j RETURN
  iptables -t nat -A OUTPUT -p tcp --dport 80 -j REDIRECT --to-ports 15001
  iptables -t nat -A OUTPUT -p tcp --dport 443 -j REDIRECT --to-ports 15001
'

# Allowed host should work for non-root.
"$RUNTIME" exec --user 1000:1000 "$NAME" sh -lc "curl -fsSIL --max-time 15 https://$ALLOW_HOST >/dev/null"

"$RUNTIME" exec --user 0 "$NAME" sh -lc '
  set -e
  pid="$(cat /tmp/ctx-egress-proxy.pid)"
  kill -0 "$pid"
  if grep -q "Too many open files" /tmp/ctx-egress-proxy.log 2>/dev/null; then
    cat /tmp/ctx-egress-proxy.log >&2 || true
    exit 1
  fi
'

# Blocked host must fail for non-root.
set +e
"$RUNTIME" exec --user 1000:1000 "$NAME" sh -lc "curl -fsSIL --max-time 15 https://$BLOCK_HOST >/dev/null"
status=$?
set -e

if [ "$status" -eq 0 ]; then
  echo "expected https://$BLOCK_HOST to be blocked, but it succeeded" >&2
  echo "--- proxy log ---" >&2
  "$RUNTIME" exec --user 0 "$NAME" sh -lc "tail -n 50 /tmp/ctx-egress-proxy.log || true" >&2
  exit 1
fi

echo "ok: allowlist enforcement works ($ALLOW_HOST allowed, $BLOCK_HOST blocked)"
