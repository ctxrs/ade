#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d /tmp/ctx-harness-egress-guard.XXXXXX)"
fake_bin="$tmp/bin"
mkdir -p "$fake_bin"

cleanup() {
  rm -rf "$tmp"
}
trap cleanup EXIT

cat >"$fake_bin/fake-runtime" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

allow_host="${ALLOW_HOST:-example.com}"
block_host="${BLOCK_HOST:-google.com}"

cmd="${1:?missing fake runtime command}"
shift || true

case "$cmd" in
  pull|run|cp|rm)
    exit 0
    ;;
  exec)
    user=""
    if [[ "${1:-}" == "--user" ]]; then
      user="$2"
      shift 2
    fi

    name="${1:-}"
    if [[ -z "$name" ]]; then
      echo "error: missing container name" >&2
      exit 1
    fi
    shift

    if [[ "${1:-}" != "sh" || "${2:-}" != "-lc" ]]; then
      echo "error: unexpected exec argv: $*" >&2
      exit 1
    fi
    shift 2

    script="${1:-}"
    if [[ -z "$script" ]]; then
      echo "error: missing exec script" >&2
      exit 1
    fi

    case "$user" in
      0)
        case "$script" in
          'command -v iptables >/dev/null 2>&1'|'test -x /usr/local/bin/ctx-egress-proxy'|'command -v curl >/dev/null 2>&1')
            exit 0
            ;;
        esac

        if [[ "$script" == *'ctx-egress-proxy --config /tmp/ctx-egress-proxy.json'* ]]; then
          exit 0
        fi

        prelude=$'iptables(){ :; }\ngetent(){ printf "127.0.0.1 localhost\\n"; }\n'
        bash -lc "$prelude$script"
        ;;
      1000:1000)
        if [[ "$script" == *"https://${allow_host}"* ]]; then
          exit 0
        fi
        if [[ "$script" == *"https://${block_host}"* ]]; then
          exit 1
        fi
        echo "error: unexpected non-root exec script: $script" >&2
        exit 1
        ;;
      *)
        echo "error: unexpected exec user: $user" >&2
        exit 1
        ;;
    esac
    ;;
  *)
    echo "error: unexpected fake runtime command: $cmd" >&2
    exit 1
    ;;
esac
EOF
chmod +x "$fake_bin/fake-runtime"

PATH="$fake_bin:$PATH" \
CONTAINER_RUNTIME=fake-runtime \
./scripts/smoke_ctx_harness_egress_guard.sh >"$tmp/smoke.log" 2>&1

if ! grep -q 'ok: allowlist enforcement works' "$tmp/smoke.log"; then
  echo "error: smoke script did not reach success path" >&2
  cat "$tmp/smoke.log" >&2
  exit 1
fi

echo "ok: harness egress smoke survives nested sh -lc quoting"
