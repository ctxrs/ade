#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ctx-provider-deps-cross-target.XXXXXX")"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

FAKE_BIN="$TMP_DIR/fake-bin"
mkdir -p "$FAKE_BIN"
cat >"$FAKE_BIN/uname" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-}" in
  -s) echo "Darwin" ;;
  -m) echo "arm64" ;;
  *) /usr/bin/uname "$@" ;;
esac
EOF
chmod +x "$FAKE_BIN/uname"

MATRIX_JSON="$TMP_DIR/provider-matrix.json"
cat >"$MATRIX_JSON" <<'EOF'
{
  "managed": {}
}
EOF

SUCCESS_OUT="$TMP_DIR/success"
PATH="$FAKE_BIN:$PATH" PROVIDER_MATRIX_JSON="$MATRIX_JSON" \
  "$ROOT_DIR/scripts/provider_deps_build_staging.sh" \
  --out-dir "$SUCCESS_OUT" \
  --os macos \
  --arch x86_64 \
  --providers ""

test -f "$SUCCESS_OUT/provider_deps_index.json"

FAIL_LOG="$TMP_DIR/fail.log"
set +e
PATH="$FAKE_BIN:$PATH" PROVIDER_MATRIX_JSON="$MATRIX_JSON" \
  "$ROOT_DIR/scripts/provider_deps_build_staging.sh" \
  --out-dir "$TMP_DIR/fail" \
  --os linux \
  --arch x86_64 \
  --providers "" >"$TMP_DIR/fail.out" 2>"$FAIL_LOG"
status=$?
set -e

if [[ "$status" -eq 0 ]]; then
  echo "expected unsupported cross-target staging to fail" >&2
  exit 1
fi

grep -q "cross-target staging is disabled" "$FAIL_LOG"
