#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 || $# -gt 3 ]]; then
  echo "usage: $0 <workspace-relative-project-dir> <entrypoint-relpath> [archive-stem]" >&2
  exit 64
fi

PROJECT_REL="$1"
ENTRYPOINT_REL="$2"
ARCHIVE_STEM="${3:-$(basename "$PROJECT_REL")}"

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

copy_project_tree() {
  local src_dir="$1"
  local dest_dir="$2"
  rm -rf "$dest_dir"
  mkdir -p "$dest_dir"
  if command -v rsync >/dev/null 2>&1; then
    rsync -a --delete \
      --exclude ".git/" \
      --exclude "node_modules/" \
      --exclude "dist/" \
      --exclude ".turbo/" \
      "$src_dir/" "$dest_dir/"
    return
  fi
  (
    cd "$src_dir"
    tar --exclude ".git" --exclude "node_modules" --exclude "dist" --exclude ".turbo" -cf - .
  ) | (
    cd "$dest_dir"
    tar -xf -
  )
}

create_deterministic_tar_gz() {
  local archive_path="$1"
  local source_dir="$2"
  shift 2
  if [[ "$#" -eq 0 ]]; then
    echo "error: create_deterministic_tar_gz requires at least one entry" >&2
    exit 1
  fi
  python3 - "$archive_path" "$source_dir" "$@" <<'PY'
import gzip
import os
import stat
import sys
import tarfile

archive_path = os.path.abspath(sys.argv[1])
source_dir = os.path.abspath(sys.argv[2])
entries = sys.argv[3:]

seen = set()
members = []

def normalize_rel(value: str) -> str:
    return value.strip().strip("/").replace("\\", "/")

def add_path(rel: str) -> None:
    rel = normalize_rel(rel)
    if not rel:
        return
    full_path = os.path.join(source_dir, rel)
    if not os.path.lexists(full_path):
        raise FileNotFoundError(f"missing archive entry: {full_path}")
    if rel in seen:
        return
    seen.add(rel)
    members.append(rel)
    st = os.lstat(full_path)
    if stat.S_ISDIR(st.st_mode):
        for child in sorted(os.listdir(full_path)):
            add_path(f"{rel}/{child}")

for entry in entries:
    add_path(entry)

members.sort()
os.makedirs(os.path.dirname(archive_path), exist_ok=True)
tmp_path = f"{archive_path}.tmp-{os.getpid()}"

with open(tmp_path, "wb") as raw_out:
    with gzip.GzipFile(fileobj=raw_out, mode="wb", mtime=0, filename="") as gz_out:
        with tarfile.open(fileobj=gz_out, mode="w", format=tarfile.PAX_FORMAT) as tar_out:
            for rel in members:
                full_path = os.path.join(source_dir, rel)
                tar_info = tar_out.gettarinfo(full_path, arcname=rel)
                tar_info.uid = 0
                tar_info.gid = 0
                tar_info.uname = "root"
                tar_info.gname = "root"
                tar_info.mtime = 0
                if tar_info.isfile():
                    with open(full_path, "rb") as src:
                        tar_out.addfile(tar_info, src)
                else:
                    tar_out.addfile(tar_info)

os.replace(tmp_path, archive_path)
PY
}

REPO_ROOT="$(resolve_repo_root)"
if [[ -z "$REPO_ROOT" ]]; then
  echo "error: failed to locate repo root for provider archive build" >&2
  exit 1
fi

PROJECT_DIR="$REPO_ROOT/$PROJECT_REL"
if [[ ! -d "$PROJECT_DIR" ]]; then
  echo "error: provider project missing: $PROJECT_DIR" >&2
  exit 1
fi

if ! command -v pnpm >/dev/null 2>&1; then
  echo "error: missing required command: pnpm" >&2
  exit 1
fi
if ! command -v python3 >/dev/null 2>&1; then
  echo "error: missing required command: python3" >&2
  exit 1
fi

TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/ctx-provider-node-archive.XXXXXX")"
WORKSPACE_DIR="$TMP_ROOT/workspace"
ARCHIVE_ROOT="${CTX_PROVIDER_NODE_ARCHIVE_OUT_DIR:-$TMP_ROOT/out}"
mkdir -p "$ARCHIVE_ROOT"

copy_project_tree "$PROJECT_DIR" "$WORKSPACE_DIR"

if [[ ! -f "$WORKSPACE_DIR/pnpm-lock.yaml" ]]; then
  echo "error: provider archive build requires pnpm-lock.yaml in $PROJECT_REL" >&2
  exit 1
fi

(
  cd "$WORKSPACE_DIR"
  pnpm install --frozen-lockfile --ignore-scripts
  pnpm run build
  pnpm install --prod --frozen-lockfile --ignore-scripts
) >&2

rm -f "$WORKSPACE_DIR/node_modules/.pnpm-workspace-state-v1.json" || true
rm -f "$WORKSPACE_DIR/node_modules/.modules.yaml" || true
find "$WORKSPACE_DIR/node_modules" -type d -name .bin -prune -exec rm -rf {} + 2>/dev/null || true

if [[ ! -e "$WORKSPACE_DIR/$ENTRYPOINT_REL" ]]; then
  echo "error: missing provider archive entrypoint after build: $PROJECT_REL/$ENTRYPOINT_REL" >&2
  exit 1
fi

archive_path="$ARCHIVE_ROOT/${ARCHIVE_STEM}.tar.gz"
create_deterministic_tar_gz "$archive_path" "$WORKSPACE_DIR" package.json dist node_modules
printf '%s\n' "$archive_path"
