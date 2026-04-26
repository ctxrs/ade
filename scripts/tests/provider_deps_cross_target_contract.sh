#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ctx-provider-deps-cross-target.XXXXXX")"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

MATRIX_JSON="$TMP_DIR/provider-matrix.json"
cat >"$MATRIX_JSON" <<'EOF'
{
  "managed": {}
}
EOF

SUCCESS_OUT="$TMP_DIR/success"
CTX_PROVIDER_DEPS_BUILD_HOST_OS="macos" \
CTX_PROVIDER_DEPS_BUILD_HOST_ARCH="aarch64" \
PROVIDER_MATRIX_JSON="$MATRIX_JSON" \
  "$ROOT_DIR/scripts/provider_deps_build_staging.sh" \
  --out-dir "$SUCCESS_OUT" \
  --os macos \
  --arch x86_64 \
  --providers ""

test -f "$SUCCESS_OUT/provider_deps_index.json"

SUCCESS_OUT_X86_TO_ARM="$TMP_DIR/success-x86-to-arm"
CTX_PROVIDER_DEPS_BUILD_HOST_OS="macos" \
CTX_PROVIDER_DEPS_BUILD_HOST_ARCH="x86_64" \
PROVIDER_MATRIX_JSON="$MATRIX_JSON" \
  "$ROOT_DIR/scripts/provider_deps_build_staging.sh" \
  --out-dir "$SUCCESS_OUT_X86_TO_ARM" \
  --os macos \
  --arch aarch64 \
  --providers ""

test -f "$SUCCESS_OUT_X86_TO_ARM/provider_deps_index.json"

FAIL_LOG="$TMP_DIR/fail.log"
set +e
CTX_PROVIDER_DEPS_BUILD_HOST_OS="macos" \
CTX_PROVIDER_DEPS_BUILD_HOST_ARCH="aarch64" \
PROVIDER_MATRIX_JSON="$MATRIX_JSON" \
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
