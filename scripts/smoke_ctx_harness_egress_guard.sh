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
# - We intentionally run curl as a non-root uid because the daemon config allows uid 0 egress.

RUNTIME="${CONTAINER_RUNTIME:-nerdctl}"
IMAGE="${CTX_HARNESS_IMAGE:-ghcr.io/ctxrs/ctx-harness:ubuntu-24.04}"

ALLOW_HOST="${ALLOW_HOST:-example.com}"
BLOCK_HOST="${BLOCK_HOST:-google.com}"

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
  "allowlist": ["$ALLOW_HOST"]
}
EOF

"$RUNTIME" cp "$CFG_LOCAL" "$NAME:/tmp/ctx-egress-proxy.json"

# Start the proxy.
"$RUNTIME" exec --user 0 "$NAME" sh -lc '
  set -e
  nohup /usr/local/bin/ctx-egress-proxy --config /tmp/ctx-egress-proxy.json >/tmp/ctx-egress-proxy.log 2>&1 &
  echo $! >/tmp/ctx-egress-proxy.pid
'

# Configure iptables the same way ctx does (with a harmless daemon allow rule).
"$RUNTIME" exec --user 0 "$NAME" sh -lc '
  set -e
  daemon_ip="$(getent hosts localhost | awk "{print \$1}" | head -n1)"
  test -n "$daemon_ip"

  iptables -t nat -F OUTPUT || true
  iptables -F OUTPUT || true
  iptables -P OUTPUT DROP
  iptables -A OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
  iptables -A OUTPUT -d 127.0.0.1/8 -j ACCEPT
  iptables -A OUTPUT -o lo -j ACCEPT
  iptables -A OUTPUT -p udp --dport 53 -j ACCEPT
  iptables -A OUTPUT -p tcp --dport 53 -j ACCEPT
  iptables -A OUTPUT -d "$daemon_ip" -p tcp --dport 9 -j ACCEPT
  iptables -A OUTPUT -m owner --uid-owner 0 -j ACCEPT

  iptables -t nat -A OUTPUT -m owner --uid-owner 0 -j RETURN
  iptables -t nat -A OUTPUT -p tcp --dport 80 -j REDIRECT --to-ports 15001
  iptables -t nat -A OUTPUT -p tcp --dport 443 -j REDIRECT --to-ports 15001
'

# Allowed host should work for non-root.
"$RUNTIME" exec --user 1000:1000 "$NAME" sh -lc "curl -fsSIL --max-time 15 https://$ALLOW_HOST >/dev/null"

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
