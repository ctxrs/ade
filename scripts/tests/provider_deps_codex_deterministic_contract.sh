#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ctx-provider-deps-codex-deterministic.XXXXXX")"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

MATRIX_JSON="$TMP_DIR/provider-matrix.json"
cat >"$MATRIX_JSON" <<'JSON'
{
  "providers": [
    {
      "id": "codex",
      "managed_install": {
        "version": "0.114.0-ctx.3"
      }
    }
  ]
}
JSON

FAKEBIN="$TMP_DIR/fakebin"
mkdir -p "$FAKEBIN"
cat >"$FAKEBIN/strip" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

target="${@: -1}"
if [[ "$(basename "$target")" != "codex-crp" ]]; then
  echo "error: expected strip to run on copied codex-crp binary, got $target" >&2
  exit 1
fi

cat >"$target" <<'EOF'
normalized-codex-crp
EOF
chmod +x "$target"
SH
chmod +x "$FAKEBIN/strip"

BIN1="$TMP_DIR/codex-bin-1"
BIN2="$TMP_DIR/codex-bin-2"
cat >"$BIN1" <<'EOF'
codex-crp build-root-a
EOF
cat >"$BIN2" <<'EOF'
codex-crp build-root-b
EOF
chmod +x "$BIN1" "$BIN2"

OUT1="$TMP_DIR/out1"
OUT2="$TMP_DIR/out2"

run_stage() {
  local out_dir="$1"
  local binary_path="$2"
  PATH="$FAKEBIN:$PATH" \
  CTX_PROVIDER_DEPS_BUILD_HOST_OS="macos" \
  CTX_PROVIDER_DEPS_BUILD_HOST_ARCH="aarch64" \
  PROVIDER_MATRIX_JSON="$MATRIX_JSON" \
  CTX_PROVIDER_DEPS_CODEX_BIN="$binary_path" \
  CTX_PROVIDER_DEPS_REQUIRE_PREBUILT=1 \
    "$ROOT_DIR/scripts/provider_deps_build_staging.sh" \
      --out-dir "$out_dir" \
      --os macos \
      --arch aarch64 \
      --providers codex >/dev/null
}

run_stage "$OUT1" "$BIN1"
run_stage "$OUT2" "$BIN2"

ARCHIVE1="$(find "$OUT1/providers" -name '*.tar.gz' -print -quit)"
ARCHIVE2="$(find "$OUT2/providers" -name '*.tar.gz' -print -quit)"

if [[ -z "$ARCHIVE1" || -z "$ARCHIVE2" ]]; then
  echo "error: expected staged codex archives from provider_deps_build_staging.sh" >&2
  exit 1
fi

SHA1="$(shasum -a 256 "$ARCHIVE1" | awk '{print $1}')"
SHA2="$(shasum -a 256 "$ARCHIVE2" | awk '{print $1}')"

if [[ "$SHA1" != "$SHA2" ]]; then
  echo "error: staged codex archives changed even after strip normalization" >&2
  echo "first:  $SHA1" >&2
  echo "second: $SHA2" >&2
  exit 1
fi

echo "ok: codex provider staging strips macOS binaries before deterministic archive creation"
