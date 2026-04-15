#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <target-key>" >&2
  exit 64
fi

TARGET_KEY="$1"
case "$TARGET_KEY" in
  darwin-aarch64|darwin-x86_64|linux-aarch64|linux-x86_64)
    ;;
  *)
    echo "error: unsupported claude-crp target key: $TARGET_KEY" >&2
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
  echo "error: failed to locate repo root for claude-crp archive build" >&2
  exit 1
fi

OUT_ROOT="${CTX_PROVIDER_CLAUDE_CRP_ARCHIVE_OUT_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/ctx-provider-claude-crp.XXXXXX")}"
STAGE_DIR="$OUT_ROOT/$TARGET_KEY"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR"

CLAUDE_CRP_OUT_DIR="$STAGE_DIR" \
CLAUDE_CRP_TARGETS="$TARGET_KEY" \
CLAUDE_CRP_VERSION="${CLAUDE_CRP_VERSION:-}" \
  "$REPO_ROOT/scripts/claude_crp_stage_archives.sh" >&2

INDEX_JSON="$STAGE_DIR/claude-crp-index.json"
if [[ ! -f "$INDEX_JSON" ]]; then
  echo "error: missing claude-crp index after staging: $INDEX_JSON" >&2
  exit 1
fi

node - "$INDEX_JSON" "$TARGET_KEY" <<'NODE'
const fs = require("node:fs");
const [indexPath, targetKey] = process.argv.slice(2);
const parsed = JSON.parse(fs.readFileSync(indexPath, "utf8"));
const artifacts = Array.isArray(parsed.artifacts) ? parsed.artifacts : [];
const hit = artifacts.find((entry) => String(entry?.target_key || "") === targetKey);
if (!hit || !hit.path) {
  process.stderr.write(`error: missing claude-crp artifact for ${targetKey} in ${indexPath}\n`);
  process.exit(1);
}
process.stdout.write(`${String(hit.path).trim()}\n`);
NODE
