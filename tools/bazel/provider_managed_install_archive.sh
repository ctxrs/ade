#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <provider-id> <target-key>" >&2
  exit 64
fi

PROVIDER_ID="$1"
TARGET_KEY="$2"

case "$TARGET_KEY" in
  darwin-aarch64)
    TARGET_OS="macos"
    TARGET_ARCH="aarch64"
    ;;
  darwin-x86_64)
    TARGET_OS="macos"
    TARGET_ARCH="x86_64"
    ;;
  linux-aarch64)
    TARGET_OS="linux"
    TARGET_ARCH="aarch64"
    ;;
  linux-x86_64)
    TARGET_OS="linux"
    TARGET_ARCH="x86_64"
    ;;
  *)
    echo "error: unsupported managed-install target key: $TARGET_KEY" >&2
    exit 2
    ;;
esac

resolve_repo_root() {
  local candidate
  for candidate in \
    "${BUILD_WORKSPACE_DIRECTORY:-}" \
    "${RUNFILES_DIR:-}/_main" \
    "${RUNFILES_DIR:-}/${TEST_WORKSPACE:-}"
  do
    if [[ -n "${candidate}" && -f "${candidate}/core/package.json" ]]; then
      printf '%s\n' "${candidate}"
      return 0
    fi
  done
  return 1
}

REPO_ROOT="$(resolve_repo_root)"
if [[ -z "$REPO_ROOT" ]]; then
  echo "error: failed to locate repo root for managed-install archive build" >&2
  exit 1
fi

OUT_ROOT="${CTX_PROVIDER_MANAGED_ARCHIVE_OUT_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/ctx-provider-managed-install.XXXXXX")}"
STAGE_DIR="$OUT_ROOT/$PROVIDER_ID-$TARGET_KEY"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR"

"$REPO_ROOT/scripts/provider_deps_build_staging.sh" \
  --out-dir "$STAGE_DIR" \
  --os "$TARGET_OS" \
  --arch "$TARGET_ARCH" \
  --providers "$PROVIDER_ID" >&2

INDEX_JSON="$STAGE_DIR/provider_deps_index.json"
if [[ ! -f "$INDEX_JSON" ]]; then
  echo "error: missing provider-deps index after managed-install staging: $INDEX_JSON" >&2
  exit 1
fi

node - "$INDEX_JSON" "$PROVIDER_ID" <<'NODE'
const fs = require("node:fs");
const [indexPath, providerId] = process.argv.slice(2);
const parsed = JSON.parse(fs.readFileSync(indexPath, "utf8"));
const providers = Array.isArray(parsed.providers) ? parsed.providers : [];
const hit = providers.find((entry) => String(entry?.provider_id || "") === providerId);
if (!hit || !hit.artifact_path) {
  process.stderr.write(`error: missing staged provider artifact for ${providerId} in ${indexPath}\n`);
  process.exit(1);
}
process.stdout.write(`${String(hit.artifact_path).trim()}\n`);
NODE
