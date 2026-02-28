#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MATRIX_JSON="${PROVIDER_MATRIX_JSON:-$ROOT/core/crates/ctx-http/src/provider_matrix.json}"
OUT_DIR=""
OS_OVERRIDE=""
ARCH_OVERRIDE=""
PROVIDERS_RAW="${CTX_PROVIDER_DEPS_BUILD_PROVIDERS:-acp-crp-bridge,amp,droid,goose,openhands,pi,claude-crp}"

usage() {
  cat <<'USAGE'
Usage: scripts/provider_deps_build_staging.sh --out-dir <dir> [options]

Options:
  --out-dir <dir>         Output staging directory (required)
  --os <linux|macos>      Explicit target OS; must match host
  --arch <x86_64|aarch64> Explicit target arch; must match host
  --providers <csv>       Provider IDs to stage

Env:
  PROVIDER_MATRIX_JSON
  CTX_PROVIDER_DEPS_BUILD_PROVIDERS
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out-dir)
      OUT_DIR="${2:-}"
      shift 2
      ;;
    --os)
      OS_OVERRIDE="${2:-}"
      shift 2
      ;;
    --arch)
      ARCH_OVERRIDE="${2:-}"
      shift 2
      ;;
    --providers)
      PROVIDERS_RAW="${2:-}"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "error: unsupported arg: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ -z "$OUT_DIR" ]]; then
  echo "error: --out-dir is required" >&2
  usage
  exit 2
fi

if [[ ! -f "$MATRIX_JSON" ]]; then
  echo "error: provider matrix missing: $MATRIX_JSON" >&2
  exit 2
fi

detect_os() {
  case "$(uname -s)" in
    Linux) echo "linux" ;;
    Darwin) echo "macos" ;;
    *) echo "unsupported"; return 1 ;;
  esac
}

detect_arch() {
  case "$(uname -m)" in
    x86_64|amd64) echo "x86_64" ;;
    aarch64|arm64) echo "aarch64" ;;
    *) echo "unsupported"; return 1 ;;
  esac
}

HOST_OS="$(detect_os)"
HOST_ARCH="$(detect_arch)"
TARGET_OS="${OS_OVERRIDE:-$HOST_OS}"
TARGET_ARCH="${ARCH_OVERRIDE:-$HOST_ARCH}"

if [[ "$TARGET_OS" != "$HOST_OS" || "$TARGET_ARCH" != "$HOST_ARCH" ]]; then
  echo "error: cross-target staging is disabled (${TARGET_OS}/${TARGET_ARCH} requested on ${HOST_OS}/${HOST_ARCH})." >&2
  echo "error: run this script on a native runner for the requested target." >&2
  exit 2
fi

case "${TARGET_OS}/${TARGET_ARCH}" in
  linux/x86_64) RUST_TARGET="x86_64-unknown-linux-gnu" ;;
  linux/aarch64) RUST_TARGET="aarch64-unknown-linux-gnu" ;;
  macos/x86_64) RUST_TARGET="x86_64-apple-darwin" ;;
  macos/aarch64) RUST_TARGET="aarch64-apple-darwin" ;;
  *)
    echo "error: unsupported target ${TARGET_OS}/${TARGET_ARCH}" >&2
    exit 2
    ;;
esac

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
entries_ndjson="$OUT_DIR/provider_deps_entries.ndjson"
: > "$entries_ndjson"

NODE_WORKSPACES_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ctx-provider-deps-node-workspaces.XXXXXX")"
cleanup() {
  rm -rf "$NODE_WORKSPACES_DIR" || true
}
trap cleanup EXIT

sha256_file() {
  local path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{print tolower($1)}'
  else
    shasum -a 256 "$path" | awk '{print tolower($1)}'
  fi
}

file_size_bytes() {
  local path="$1"
  if stat -f '%z' "$path" >/dev/null 2>&1; then
    stat -f '%z' "$path"
  else
    stat -c '%s' "$path"
  fi
}

require_cmd() {
  local name="$1"
  if ! command -v "$name" >/dev/null 2>&1; then
    echo "error: missing required command: $name" >&2
    exit 1
  fi
}

resolve_python_cmd() {
  if command -v python3 >/dev/null 2>&1; then
    echo "python3"
    return
  fi
  if command -v python >/dev/null 2>&1; then
    echo "python"
    return
  fi
  echo "error: missing required command: python3 (or python)" >&2
  exit 1
}

PYTHON_CMD="${PYTHON_CMD:-$(resolve_python_cmd)}"

run_python() {
  "$PYTHON_CMD" "$@"
}

path_hash() {
  local value="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    printf '%s' "$value" | sha256sum | awk '{print substr($1,1,12)}'
  else
    printf '%s' "$value" | shasum -a 256 | awk '{print substr($1,1,12)}'
  fi
}

copy_node_project_to_workspace() {
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
  run_python - "$archive_path" "$source_dir" "$@" <<'PY'
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

get_provider_version() {
  local provider_id="$1"
  node - "$MATRIX_JSON" "$provider_id" <<'NODE'
const fs = require("node:fs");
const [matrixPath, providerId] = process.argv.slice(2);
const matrix = JSON.parse(fs.readFileSync(matrixPath, "utf8"));
const provider = (matrix.providers || []).find((entry) => entry?.id === providerId);
if (!provider) process.exit(1);
const managed = provider.managed_install || {};
let version = String(managed.version || "").trim();
if (!version) {
  const releases = Array.isArray(provider.releases) ? provider.releases : [];
  const supported = releases.filter((rel) => rel && rel.status !== "blocked" && typeof rel.version === "string");
  if (supported.length > 0) version = String(supported[0].version || "").trim();
}
if (!version) process.exit(2);
process.stdout.write(version);
NODE
}

write_entry() {
  local provider_id="$1"
  local version="$2"
  local archive_kind="$3"
  local bin_path="$4"
  local artifact_path="$5"
  local sha256="$6"
  local size_bytes="$7"
  local filename
  filename="$(basename "$artifact_path")"
  local artifact_rel_path="$artifact_path"
  if [[ "$artifact_rel_path" == "$OUT_DIR/"* ]]; then
    artifact_rel_path="${artifact_rel_path#"$OUT_DIR/"}"
  fi
  node - "$provider_id" "$version" "$archive_kind" "$bin_path" "$artifact_path" "$artifact_rel_path" "$sha256" "$filename" "$TARGET_OS" "$TARGET_ARCH" "$size_bytes" >> "$entries_ndjson" <<'NODE'
const [providerId, version, archiveKind, binPath, artifactPath, artifactRelPath, sha256, filename, os, arch, sizeBytes] =
  process.argv.slice(2);
const parsedSize = Number(sizeBytes);
process.stdout.write(
  `${JSON.stringify({
    provider_id: providerId,
    version,
    os,
    arch,
    archive: archiveKind,
    bin_path: binPath,
    artifact_path: artifactPath,
    artifact_rel_path: artifactRelPath,
    filename,
    sha256,
    size_bytes: Number.isFinite(parsedSize) && parsedSize > 0 ? Math.floor(parsedSize) : null,
  })}\n`,
);
NODE
}

stage_archive_from_binary() {
  local provider_id="$1"
  local version="$2"
  local binary_path="$3"
  local out_name="$4"
  local stage_dir="$OUT_DIR/providers/$provider_id/$version/$TARGET_OS/$TARGET_ARCH"
  mkdir -p "$stage_dir"
  local tmp_dir
  tmp_dir="$(mktemp -d "$OUT_DIR/.pkg-${provider_id}.XXXXXX")"
  cp "$binary_path" "$tmp_dir/$out_name"
  chmod +x "$tmp_dir/$out_name" || true
  local archive_path="$stage_dir/${provider_id}-${version}-${TARGET_OS}-${TARGET_ARCH}.tar.gz"
  create_deterministic_tar_gz "$archive_path" "$tmp_dir" "$out_name"
  rm -rf "$tmp_dir"
  echo "$archive_path"
}

stage_archive_from_node_project() {
  local provider_id="$1"
  local version="$2"
  local workspace_dir="$3"
  local entrypoint_rel="$4"
  local stage_dir="$OUT_DIR/providers/$provider_id/$version/$TARGET_OS/$TARGET_ARCH"
  mkdir -p "$stage_dir"

  if [[ ! -e "$workspace_dir/$entrypoint_rel" ]]; then
    echo "error: missing node entrypoint for $provider_id: $workspace_dir/$entrypoint_rel" >&2
    exit 1
  fi

  local root_entries=(package.json dist node_modules)
  local entry_dir="${entrypoint_rel%%/*}"
  if [[ "$entry_dir" != "dist" && "$entry_dir" != "node_modules" && "$entry_dir" != "package.json" ]]; then
    root_entries+=("$entry_dir")
  fi

  local archive_path="$stage_dir/${provider_id}-${version}-${TARGET_OS}-${TARGET_ARCH}.tar.gz"
  create_deterministic_tar_gz "$archive_path" "$workspace_dir" "${root_entries[@]}"
  echo "$archive_path"
}

prepare_node_workspace() {
  local project_dir="$1"
  local workspace_key
  workspace_key="$(path_hash "$project_dir")"
  local workspace_dir="$NODE_WORKSPACES_DIR/$(basename "$project_dir")-$workspace_key"
  local ready_marker="$workspace_dir/.ctx_node_stage_ready"

  if [[ ! -f "$ready_marker" ]]; then
    require_cmd pnpm
    copy_node_project_to_workspace "$project_dir" "$workspace_dir"
    if [[ ! -f "$workspace_dir/pnpm-lock.yaml" ]]; then
      echo "error: node provider workspace missing pnpm-lock.yaml: $project_dir" >&2
      exit 1
    fi
    (
      cd "$workspace_dir"
      pnpm install --frozen-lockfile --ignore-scripts
      pnpm run build
      pnpm install --prod --frozen-lockfile --ignore-scripts
    ) >&2
    rm -f "$workspace_dir/node_modules/.pnpm-workspace-state-v1.json" || true
    rm -f "$workspace_dir/node_modules/.modules.yaml" || true
    find "$workspace_dir/node_modules" -type d -name .bin -prune -exec rm -rf {} + 2>/dev/null || true
    touch "$ready_marker"
  fi

  echo "$workspace_dir"
}

resolve_rust_target_root() {
  local project_dir="$1"
  local configured_target_dir="${CARGO_TARGET_DIR:-}"
  if [[ -z "$configured_target_dir" ]]; then
    echo "$project_dir/target"
    return
  fi
  case "$configured_target_dir" in
    /*) echo "$configured_target_dir" ;;
    *) echo "$project_dir/$configured_target_dir" ;;
  esac
}

build_rust_provider() {
  local provider_id="$1"
  local project_dir="$2"
  local binary_name="$3"
  local version="$4"
  local package_name="${5:-$binary_name}"
  local rustflags="${RUSTFLAGS:-}"
  if [[ -n "$rustflags" ]]; then
    rustflags="$rustflags -C debuginfo=0"
  else
    rustflags="-C debuginfo=0"
  fi

  require_cmd cargo
  (
    cd "$project_dir"
    CARGO_INCREMENTAL=0 \
    CARGO_PROFILE_RELEASE_DEBUG=0 \
    CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO=off \
    RUSTFLAGS="$rustflags" \
    cargo build --release --target "$RUST_TARGET" --package "$package_name" --bin "$binary_name"
  ) >&2

  local target_root
  target_root="$(resolve_rust_target_root "$project_dir")"
  local bin="$target_root/$RUST_TARGET/release/$binary_name"
  if [[ ! -f "$bin" ]]; then
    echo "error: missing built binary for $provider_id at $bin" >&2
    exit 1
  fi

  local archive_path
  archive_path="$(stage_archive_from_binary "$provider_id" "$version" "$bin" "$binary_name")"
  local sha
  sha="$(sha256_file "$archive_path")"
  local size_bytes
  size_bytes="$(file_size_bytes "$archive_path")"
  write_entry "$provider_id" "$version" "tar_gz" "$binary_name" "$archive_path" "$sha" "$size_bytes"
}

build_node_project_provider() {
  local provider_id="$1"
  local project_dir="$2"
  local entrypoint_rel="$3"
  local version="$4"

  local workspace_dir
  workspace_dir="$(prepare_node_workspace "$project_dir")"
  chmod +x "$workspace_dir/$entrypoint_rel" || true
  local archive_path
  archive_path="$(stage_archive_from_node_project "$provider_id" "$version" "$workspace_dir" "$entrypoint_rel")"

  local sha
  sha="$(sha256_file "$archive_path")"
  local size_bytes
  size_bytes="$(file_size_bytes "$archive_path")"
  write_entry "$provider_id" "$version" "tar_gz" "$entrypoint_rel" "$archive_path" "$sha" "$size_bytes"
}

build_claude_crp_provider() {
  local version="$1"
  local claude_target_os="$TARGET_OS"
  if [[ "$claude_target_os" == "macos" ]]; then
    claude_target_os="darwin"
  fi
  local tmp_dir
  tmp_dir="$(mktemp -d "$OUT_DIR/.claude-crp-stage.XXXXXX")"
  CLAUDE_CRP_OUT_DIR="$tmp_dir" \
  CLAUDE_CRP_TARGETS="${claude_target_os}-${TARGET_ARCH}" \
  CLAUDE_CRP_VERSION="$version" \
  "$ROOT/scripts/claude_crp_stage_archives.sh" >&2

  local index_json="$tmp_dir/claude-crp-index.json"
  if [[ ! -f "$index_json" ]]; then
    echo "error: claude-crp staging did not produce index: $index_json" >&2
    exit 1
  fi

  local artifact_path
  artifact_path="$(node - "$index_json" "$claude_target_os" "$TARGET_ARCH" <<'NODE'
const fs = require("node:fs");
const [indexPath, os, arch] = process.argv.slice(2);
const parsed = JSON.parse(fs.readFileSync(indexPath, "utf8"));
const artifacts = Array.isArray(parsed.artifacts) ? parsed.artifacts : [];
const hit = artifacts.find((entry) => String(entry?.os || "") === os && String(entry?.arch || "") === arch);
if (!hit || !hit.path) process.exit(1);
process.stdout.write(String(hit.path));
NODE
)"
  if [[ -z "$artifact_path" || ! -f "$artifact_path" ]]; then
    echo "error: missing claude-crp artifact for ${TARGET_OS}/${TARGET_ARCH} from $index_json" >&2
    exit 1
  fi

  local stage_dir="$OUT_DIR/providers/claude-crp/$version/$TARGET_OS/$TARGET_ARCH"
  mkdir -p "$stage_dir"
  local staged_archive="$stage_dir/$(basename "$artifact_path")"
  cp "$artifact_path" "$staged_archive"

  local sha
  sha="$(sha256_file "$staged_archive")"
  local size_bytes
  size_bytes="$(file_size_bytes "$staged_archive")"
  write_entry "claude-crp" "$version" "tar_gz" "bin/claude-crp" "$staged_archive" "$sha" "$size_bytes"

  rm -rf "$tmp_dir"
}

IFS=',' read -r -a providers <<< "$PROVIDERS_RAW"
for provider in "${providers[@]}"; do
  provider="$(echo "$provider" | xargs)"
  [[ -n "$provider" ]] || continue
  version="$(get_provider_version "$provider")"
  case "$provider" in
    acp-crp-bridge)
      build_rust_provider "$provider" "$ROOT/external-harnesses/acp-crp-bridge" "acp-crp-bridge" "$version" "acp-crp-bridge"
      ;;
    droid)
      build_rust_provider "$provider" "$ROOT/harness-adapters/droid-acp" "droid-acp" "$version" "droid-acp"
      ;;
    amp)
      build_node_project_provider "$provider" "$ROOT/harness-adapters/example-acp" "dist/bin/amp-acp.js" "$version"
      ;;
    pi)
      build_node_project_provider "$provider" "$ROOT/harness-adapters/pi-acp" "dist/bin/pi-acp.js" "$version"
      ;;
    openhands)
      build_node_project_provider "$provider" "$ROOT/harness-adapters/openhands-acp" "dist/bin/openhands-acp.js" "$version"
      ;;
    goose)
      build_node_project_provider "$provider" "$ROOT/harness-adapters/openhands-acp" "dist/bin/goose-acp.js" "$version"
      ;;
    claude-crp)
      build_claude_crp_provider "$version"
      ;;
    *)
      echo "error: provider not supported by build script yet: $provider" >&2
      exit 2
      ;;
  esac
done

node - "$entries_ndjson" "$OUT_DIR/provider_deps_index.json" <<'NODE'
const fs = require("node:fs");
const [entriesPath, outPath] = process.argv.slice(2);
const lines = fs
  .readFileSync(entriesPath, "utf8")
  .split(/\r?\n/)
  .map((line) => line.trim())
  .filter(Boolean);
const entries = lines.map((line) => JSON.parse(line));
const payload = {
  version: 1,
  generated_at: new Date().toISOString(),
  providers: entries,
};
fs.writeFileSync(outPath, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
NODE

echo "provider deps staged at $OUT_DIR"
echo "index: $OUT_DIR/provider_deps_index.json"
