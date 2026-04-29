#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ctx-provider-deps-cross-target.XXXXXX")"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT
export CTX_VOLATILE_ROOT="$TMP_DIR/volatile"
export CARGO_TARGET_DIR="$TMP_DIR/volatile/targets/cargo"
export CTX_VERIFY_CARGO_TARGET_DIR="$CARGO_TARGET_DIR"

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
  bash "$ROOT_DIR/scripts/provider_deps_build_staging.sh" \
  --out-dir "$SUCCESS_OUT" \
  --os macos \
  --arch x86_64 \
  --providers ""

test -f "$SUCCESS_OUT/provider_deps_index.json"

SUCCESS_OUT_X86_TO_ARM="$TMP_DIR/success-x86-to-arm"
CTX_PROVIDER_DEPS_BUILD_HOST_OS="macos" \
CTX_PROVIDER_DEPS_BUILD_HOST_ARCH="x86_64" \
PROVIDER_MATRIX_JSON="$MATRIX_JSON" \
  bash "$ROOT_DIR/scripts/provider_deps_build_staging.sh" \
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
  bash "$ROOT_DIR/scripts/provider_deps_build_staging.sh" \
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

PYTHON_PROVIDER_MATRIX_JSON="$TMP_DIR/python-provider-matrix.json"
cat >"$PYTHON_PROVIDER_MATRIX_JSON" <<'EOF'
{
  "providers": [
    {
      "id": "openhands",
      "managed_install": {
        "kind": "python",
        "package": "openhands",
        "entrypoint": "openhands",
        "version": "1.14.0",
        "python_version": "3.12.13",
        "python_build_tag": "20260303"
      }
    }
  ]
}
EOF

FAKE_BIN="$TMP_DIR/fake-bin"
FAKE_PYTHON_LOG="$TMP_DIR/fake-python.log"
mkdir -p "$FAKE_BIN"
cat >"$FAKE_BIN/python3.12" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

printf '%s\n' "$*" >>"${FAKE_PYTHON_LOG:?}"

if [[ "${1:-}" == "-c" ]]; then
  printf '3.12\n'
  exit 0
fi

if [[ "${1:-}" == "-m" && "${2:-}" == "pip" && "${3:-}" == "wheel" ]]; then
  wheel_dir=""
  shift 3
  while [[ "$#" -gt 0 ]]; do
    case "$1" in
      --wheel-dir)
        wheel_dir="${2:-}"
        shift 2
        ;;
      *)
        shift
        ;;
    esac
  done
  mkdir -p "$wheel_dir"
  : >"$wheel_dir/func_timeout-4.3.5-py3-none-any.whl"
  exit 0
fi

if [[ "${1:-}" == "-m" && "${2:-}" == "pip" && "${3:-}" == "install" ]]; then
  target_dir=""
  shift 3
  while [[ "$#" -gt 0 ]]; do
    case "$1" in
      --target)
        target_dir="${2:-}"
        shift 2
        ;;
      *)
        shift
        ;;
    esac
  done
  mkdir -p "$target_dir"
  exit 0
fi

echo "unexpected fake python invocation: $*" >&2
exit 2
EOF
chmod +x "$FAKE_BIN/python3.12"

PYTHON_PROVIDER_OUT="$TMP_DIR/python-provider"
TARGET_RUNTIME="$PYTHON_PROVIDER_OUT/.python-runtimes/target/cpython-3.12.13+20260303-x86_64-apple-darwin"
mkdir -p "$TARGET_RUNTIME/bin"
cat >"$TARGET_RUNTIME/bin/python3" <<'EOF'
#!/usr/bin/env bash
echo "target python should not run during cross-target staging" >&2
exit 2
EOF
chmod +x "$TARGET_RUNTIME/bin/python3"

CTX_PROVIDER_DEPS_BUILD_HOST_OS="macos" \
CTX_PROVIDER_DEPS_BUILD_HOST_ARCH="aarch64" \
FAKE_PYTHON_LOG="$FAKE_PYTHON_LOG" \
PATH="$FAKE_BIN:$PATH" \
PROVIDER_MATRIX_JSON="$PYTHON_PROVIDER_MATRIX_JSON" \
  bash "$ROOT_DIR/scripts/provider_deps_build_staging.sh" \
  --out-dir "$PYTHON_PROVIDER_OUT" \
  --os macos \
  --arch x86_64 \
  --providers "openhands"

test -f "$PYTHON_PROVIDER_OUT/provider_deps_index.json"
grep -q -- "pip wheel" "$FAKE_PYTHON_LOG"
grep -q -- "func-timeout==4.3.5" "$FAKE_PYTHON_LOG"
grep -q -- "--find-links" "$FAKE_PYTHON_LOG"
grep -q -- "--only-binary=:all:" "$FAKE_PYTHON_LOG"
grep -q -- "--platform macosx_10_15_x86_64" "$FAKE_PYTHON_LOG"

NATIVE_PYTHON_PROVIDER_OUT="$TMP_DIR/python-provider-native"
NATIVE_TARGET_RUNTIME="$NATIVE_PYTHON_PROVIDER_OUT/.python-runtimes/target/cpython-3.12.13+20260303-x86_64-apple-darwin"
mkdir -p "$NATIVE_TARGET_RUNTIME/bin"
cat >"$NATIVE_TARGET_RUNTIME/bin/python3" <<'EOF'
#!/usr/bin/env bash
echo "native target python should not resolve macOS wheels implicitly" >&2
exit 2
EOF
chmod +x "$NATIVE_TARGET_RUNTIME/bin/python3"

CTX_PROVIDER_DEPS_BUILD_HOST_OS="macos" \
CTX_PROVIDER_DEPS_BUILD_HOST_ARCH="x86_64" \
FAKE_PYTHON_LOG="$FAKE_PYTHON_LOG" \
PATH="$FAKE_BIN:$PATH" \
PROVIDER_MATRIX_JSON="$PYTHON_PROVIDER_MATRIX_JSON" \
  bash "$ROOT_DIR/scripts/provider_deps_build_staging.sh" \
  --out-dir "$NATIVE_PYTHON_PROVIDER_OUT" \
  --os macos \
  --arch x86_64 \
  --providers "openhands"

test -f "$NATIVE_PYTHON_PROVIDER_OUT/provider_deps_index.json"
grep -q -- "--platform macosx_10_15_x86_64" "$FAKE_PYTHON_LOG"
