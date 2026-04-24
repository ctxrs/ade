#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MATRIX_JSON="${PROVIDER_MATRIX_JSON:-$ROOT/core/crates/ctx-provider-accounts/src/provider_matrix.json}"
OUT_DIR=""
OS_OVERRIDE=""
ARCH_OVERRIDE=""
PROVIDERS_RAW="${CTX_PROVIDER_DEPS_BUILD_PROVIDERS:-acp-crp-bridge,amp,auggie,claude-cli,claude-crp,cline,codex-cli,codex-crp,copilot,cursor,droid,gemini,goose,kimi,mistral,opencode,openhands,pi,qwen}"

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

cross_target_allowed() {
  case "${HOST_OS}/${HOST_ARCH}->${TARGET_OS}/${TARGET_ARCH}" in
    "macos/aarch64->macos/x86_64") return 0 ;;
    *) return 1 ;;
  esac
}

HOST_OS="${CTX_PROVIDER_DEPS_BUILD_HOST_OS:-$(detect_os)}"
HOST_ARCH="${CTX_PROVIDER_DEPS_BUILD_HOST_ARCH:-$(detect_arch)}"
TARGET_OS="${OS_OVERRIDE:-$HOST_OS}"
TARGET_ARCH="${ARCH_OVERRIDE:-$HOST_ARCH}"

if [[ "$TARGET_OS" != "$HOST_OS" || "$TARGET_ARCH" != "$HOST_ARCH" ]]; then
  if cross_target_allowed; then
    echo "info: allowing supported cross-target staging for ${TARGET_OS}/${TARGET_ARCH} on ${HOST_OS}/${HOST_ARCH}" >&2
  else
    echo "error: cross-target staging is disabled (${TARGET_OS}/${TARGET_ARCH} requested on ${HOST_OS}/${HOST_ARCH})." >&2
    echo "error: run this script on a native runner for the requested target." >&2
    exit 2
  fi
fi

case "${TARGET_OS}/${TARGET_ARCH}" in
  linux/x86_64) RUST_TARGET="x86_64-unknown-linux-gnu"; PYTHON_TARGET="x86_64-unknown-linux-gnu" ;;
  linux/aarch64) RUST_TARGET="aarch64-unknown-linux-gnu"; PYTHON_TARGET="aarch64-unknown-linux-gnu" ;;
  macos/x86_64) RUST_TARGET="x86_64-apple-darwin"; PYTHON_TARGET="x86_64-apple-darwin" ;;
  macos/aarch64) RUST_TARGET="aarch64-apple-darwin"; PYTHON_TARGET="aarch64-apple-darwin" ;;
  *)
    echo "error: unsupported target ${TARGET_OS}/${TARGET_ARCH}" >&2
    exit 2
    ;;
esac

export CTX_SESSION_ID="${CTX_SESSION_ID:-provider-deps-staging-${TARGET_OS}-${TARGET_ARCH}-${$}}"
eval "$(node "$ROOT/core/scripts/print_ctx_cache_env.cjs" --mode workspace --cwd "$ROOT/core" --format shell --mkdir)"

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

get_archive_target_metadata() {
  local provider_id="$1"
  node - "$MATRIX_JSON" "$provider_id" "$TARGET_OS" "$TARGET_ARCH" <<'NODE'
const fs = require("node:fs");
const [matrixPath, providerId, os, arch] = process.argv.slice(2);
const matrix = JSON.parse(fs.readFileSync(matrixPath, "utf8"));
const provider = (matrix.providers || []).find((entry) => entry?.id === providerId);
if (!provider) {
  process.exit(1);
}
const managed = provider.managed_install || {};
if (managed.kind !== "archive") {
  process.exit(2);
}
const targets = managed.targets || {};
const keys = [
  `${os}-${arch}`,
  os === "macos" ? `darwin-${arch}` : null,
].filter(Boolean);
const target = keys.map((key) => targets[key]).find(Boolean);
if (!target) {
  process.exit(3);
}
const out = [
  String(target.url || "").trim(),
  String(target.archive || "").trim(),
  String(target.bin_path || "").trim(),
  String(target.sha256 || "").trim(),
];
if (!out[0] || !out[1] || !out[2]) {
  process.exit(4);
}
process.stdout.write(`${out.join("\t")}\n`);
NODE
}

stage_matrix_archive_provider() {
  local provider_id="$1"
  local version="$2"
  local metadata
  metadata="$(get_archive_target_metadata "$provider_id")"
  local url archive_kind bin_path expected_sha
  IFS=$'\t' read -r url archive_kind bin_path expected_sha <<< "$metadata"

  local prebuilt_archive
  prebuilt_archive="$(resolve_prebuilt_provider_archive "$provider_id" || true)"
  if [[ -n "$prebuilt_archive" ]]; then
    local staged_archive
    staged_archive="$(stage_archive_from_prebuilt_archive \
      "$provider_id" \
      "$version" \
      "$prebuilt_archive" \
      "$(basename "$prebuilt_archive")")"
    local sha
    sha="$(sha256_file "$staged_archive")"
    if [[ -n "$expected_sha" && "$sha" != "$expected_sha" ]]; then
      echo "error: sha256 mismatch for $provider_id archive (expected $expected_sha, got $sha)" >&2
      exit 4
    fi
    local size_bytes
    size_bytes="$(file_size_bytes "$staged_archive")"
    write_entry "$provider_id" "$version" "$archive_kind" "$bin_path" "$staged_archive" "$sha" "$size_bytes"
    return 0
  fi
  if [[ "${CTX_PROVIDER_DEPS_REQUIRE_PREBUILT:-0}" == "1" ]]; then
    require_prebuilt_provider_archive "$provider_id"
  fi

  require_cmd curl

  local filename
  filename="$(basename "${url%%\?*}")"
  local staged_archive="$OUT_DIR/$filename"
  curl --fail --location --silent --show-error "$url" --output "$staged_archive"

  local sha
  sha="$(sha256_file "$staged_archive")"
  if [[ -n "$expected_sha" && "$sha" != "$expected_sha" ]]; then
    echo "error: sha256 mismatch for $provider_id archive (expected $expected_sha, got $sha)" >&2
    exit 4
  fi
  local size_bytes
  size_bytes="$(file_size_bytes "$staged_archive")"
  write_entry "$provider_id" "$version" "$archive_kind" "$bin_path" "$staged_archive" "$sha" "$size_bytes"
}

venv_exe() {
  local venv_dir="$1"
  local tool_name="$2"
  if [[ "$TARGET_OS" == "windows" ]]; then
    echo "$venv_dir/Scripts/${tool_name}.exe"
  else
    echo "$venv_dir/bin/$tool_name"
  fi
}

get_python_provider_metadata() {
  local provider_id="$1"
  node - "$MATRIX_JSON" "$provider_id" <<'NODE'
const fs = require("node:fs");
const [matrixPath, providerId] = process.argv.slice(2);
const matrix = JSON.parse(fs.readFileSync(matrixPath, "utf8"));
const provider = (matrix.providers || []).find((entry) => entry?.id === providerId);
if (!provider) {
  process.exit(1);
}
const managed = provider.managed_install || {};
if (managed.kind !== "python") {
  process.exit(2);
}
const out = [
  String(managed.package || "").trim(),
  String(managed.entrypoint || "").trim(),
  String(managed.version || "").trim(),
  String(managed.python_version || "").trim(),
  String(managed.python_build_tag || "").trim(),
];
if (!out[0] || !out[1] || !out[2] || !out[3] || !out[4]) {
  process.exit(3);
}
process.stdout.write(`${out.join("\t")}\n`);
NODE
}

get_npm_provider_metadata() {
  local provider_id="$1"
  node - "$MATRIX_JSON" "$provider_id" <<'NODE'
const fs = require("node:fs");
const [matrixPath, providerId] = process.argv.slice(2);
const matrix = JSON.parse(fs.readFileSync(matrixPath, "utf8"));
const provider = (matrix.providers || []).find((entry) => entry?.id === providerId);
if (!provider) {
  process.exit(1);
}
const managed = provider.managed_install || {};
if (managed.kind !== "npm") {
  process.exit(2);
}
const out = [
  String(managed.package || "").trim(),
  String(managed.entrypoint || "").trim(),
];
if (!out[0] || !out[1]) {
  process.exit(3);
}
process.stdout.write(`${out.join("\t")}\n`);
NODE
}

ensure_python_runtime() {
  local python_version="$1"
  local python_build_tag="$2"
  local runtime_root="$OUT_DIR/.python-runtimes/cpython-${python_version}+${python_build_tag}-${PYTHON_TARGET}"
  local python_bin
  python_bin="$runtime_root/bin/python3"
  if [[ ! -f "$python_bin" ]]; then
    python_bin="$runtime_root/bin/python"
  fi
  if [[ -f "$python_bin" ]]; then
    echo "$runtime_root"
    return
  fi

  require_cmd curl
  require_cmd tar

  mkdir -p "$(dirname "$runtime_root")"
  local asset="cpython-${python_version}+${python_build_tag}-${PYTHON_TARGET}-install_only.tar.gz"
  local url="https://github.com/indygreg/python-build-standalone/releases/download/${python_build_tag}/${asset}"
  local archive_tmp
  archive_tmp="$(mktemp "$OUT_DIR/.python-runtime.XXXXXX")"
  curl --fail --location --silent --show-error "$url" --output "$archive_tmp"

  local extract_dir
  extract_dir="$(mktemp -d "$OUT_DIR/.python-runtime-extract.XXXXXX")"
  tar -xzf "$archive_tmp" -C "$extract_dir"
  rm -f "$archive_tmp"

  local extracted="$extract_dir/python"
  if [[ ! -d "$extracted" ]]; then
    echo "error: python runtime extraction failed for ${python_version}+${python_build_tag}" >&2
    exit 1
  fi

  rm -rf "$runtime_root"
  mv "$extracted" "$runtime_root"
  rm -rf "$extract_dir"
  echo "$runtime_root"
}

stage_matrix_python_provider() {
  local provider_id="$1"
  local metadata
  metadata="$(get_python_provider_metadata "$provider_id")"
  local package entrypoint version python_version python_build_tag
  IFS=$'\t' read -r package entrypoint version python_version python_build_tag <<< "$metadata"

  local prebuilt_archive
  prebuilt_archive="$(resolve_prebuilt_provider_archive "$provider_id" || true)"
  if [[ -n "$prebuilt_archive" ]]; then
    local staged_archive
    staged_archive="$(stage_archive_from_prebuilt_archive \
      "$provider_id" \
      "$version" \
      "$prebuilt_archive" \
      "$(basename "$prebuilt_archive")")"
    local sha
    sha="$(sha256_file "$staged_archive")"
    local size_bytes
    size_bytes="$(file_size_bytes "$staged_archive")"
    write_entry "$provider_id" "$version" "tar_gz" "bin/${entrypoint}" "$staged_archive" "$sha" "$size_bytes"
    return 0
  fi
  if [[ "${CTX_PROVIDER_DEPS_REQUIRE_PREBUILT:-0}" == "1" ]]; then
    require_prebuilt_provider_archive "$provider_id"
  fi

  local runtime_root
  runtime_root="$(ensure_python_runtime "$python_version" "$python_build_tag")"
  local runtime_python="$runtime_root/bin/python3"
  if [[ ! -f "$runtime_python" ]]; then
    runtime_python="$runtime_root/bin/python"
  fi
  if [[ ! -f "$runtime_python" ]]; then
    echo "error: missing bundled python runtime for $provider_id at $runtime_root" >&2
    exit 1
  fi

  local provider_root
  provider_root="$(mktemp -d "$OUT_DIR/.python-provider-${provider_id}.XXXXXX")"
  mkdir -p "$provider_root/bin" "$provider_root/site-packages"
  if command -v rsync >/dev/null 2>&1; then
    rsync -a "$runtime_root/" "$provider_root/python/"
  else
    mkdir -p "$provider_root/python"
    (
      cd "$runtime_root"
      tar -cf - .
    ) | (
      cd "$provider_root/python"
      tar -xf -
    )
  fi

  local staged_python="$provider_root/python/bin/python3"
  if [[ ! -f "$staged_python" ]]; then
    staged_python="$provider_root/python/bin/python"
  fi
  if [[ ! -f "$staged_python" ]]; then
    echo "error: missing staged python binary for $provider_id: $provider_root/python" >&2
    exit 1
  fi

  PIP_DISABLE_PIP_VERSION_CHECK=1 \
  PIP_NO_INPUT=1 \
  "$staged_python" -m pip install --disable-pip-version-check --no-input --target "$provider_root/site-packages" "${package}==${version}" >&2

  local entrypoint_rel="bin/${entrypoint}"
  local entrypoint_path="$provider_root/$entrypoint_rel"
  cat > "$entrypoint_path" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export PYTHONNOUSERSITE=1
export PYTHONPATH="$script_dir/../site-packages${PYTHONPATH:+:$PYTHONPATH}"
python_bin="$script_dir/../python/bin/python3"
if [[ ! -x "$python_bin" ]]; then
  python_bin="$script_dir/../python/bin/python"
fi
entrypoint_name="__CTX_ENTRYPOINT_NAME__"
exec "$python_bin" - "$entrypoint_name" "$@" <<'PY'
import importlib.metadata
import sys

entrypoint_name = sys.argv[1]
argv = [entrypoint_name, *sys.argv[2:]]
matches = [
    entry
    for entry in importlib.metadata.entry_points(group="console_scripts")
    if entry.name == entrypoint_name
]
if not matches:
    raise SystemExit(f"missing console script entrypoint: {entrypoint_name}")

sys.argv = argv
raise SystemExit(matches[0].load()())
PY
SH
  perl -0pi -e "s/__CTX_ENTRYPOINT_NAME__/${entrypoint//\//\\/}/g" "$entrypoint_path"
  chmod +x "$entrypoint_path"

  local stage_dir="$OUT_DIR/providers/$provider_id/$version/$TARGET_OS/$TARGET_ARCH"
  mkdir -p "$stage_dir"
  local archive_path="$stage_dir/${provider_id}-${version}-${TARGET_OS}-${TARGET_ARCH}.tar.gz"
  create_deterministic_tar_gz "$archive_path" "$provider_root" "bin" "python" "site-packages"
  local sha
  sha="$(sha256_file "$archive_path")"
  local size_bytes
  size_bytes="$(file_size_bytes "$archive_path")"
  write_entry "$provider_id" "$version" "tar_gz" "$entrypoint_rel" "$archive_path" "$sha" "$size_bytes"

  rm -rf "$provider_root"
}

stage_matrix_npm_provider() {
  local provider_id="$1"
  local version="$2"
  local metadata
  metadata="$(get_npm_provider_metadata "$provider_id")"
  local package entrypoint
  IFS=$'\t' read -r package entrypoint <<< "$metadata"

  local prebuilt_archive
  prebuilt_archive="$(resolve_prebuilt_provider_archive "$provider_id" || true)"
  if [[ -n "$prebuilt_archive" ]]; then
    local staged_archive
    staged_archive="$(stage_archive_from_prebuilt_archive \
      "$provider_id" \
      "$version" \
      "$prebuilt_archive" \
      "${provider_id}-${version}-${TARGET_OS}-${TARGET_ARCH}.tar.gz")"
    local sha
    sha="$(sha256_file "$staged_archive")"
    local size_bytes
    size_bytes="$(file_size_bytes "$staged_archive")"
    write_entry "$provider_id" "$version" "tar_gz" "$entrypoint" "$staged_archive" "$sha" "$size_bytes"
    return 0
  fi
  if [[ "${CTX_PROVIDER_DEPS_REQUIRE_PREBUILT:-0}" == "1" ]]; then
    require_prebuilt_provider_archive "$provider_id"
  fi

  require_cmd pnpm
  local workspace_dir
  workspace_dir="$(mktemp -d "$OUT_DIR/.npm-provider-${provider_id}.XXXXXX")"
  cat > "$workspace_dir/package.json" <<JSON
{
  "name": "ctx-provider-deps-${provider_id}",
  "private": true,
  "version": "0.0.0"
}
JSON
  (
    cd "$workspace_dir"
    pnpm add --ignore-scripts "${package}@${version}"
  ) >&2
  rm -f "$workspace_dir/node_modules/.pnpm-workspace-state-v1.json" || true
  rm -f "$workspace_dir/node_modules/.modules.yaml" || true
  find "$workspace_dir/node_modules" -type d -name .bin -prune -exec rm -rf {} + 2>/dev/null || true

  local archive_path
  archive_path="$(stage_archive_from_node_project "$provider_id" "$version" "$workspace_dir" "$entrypoint")"
  local sha
  sha="$(sha256_file "$archive_path")"
  local size_bytes
  size_bytes="$(file_size_bytes "$archive_path")"
  write_entry "$provider_id" "$version" "tar_gz" "$entrypoint" "$archive_path" "$sha" "$size_bytes"

  rm -rf "$workspace_dir"
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

stage_codex_archive_from_binary() {
  local version="$1"
  local binary_path="$2"
  local stage_dir="$OUT_DIR/providers/codex-crp/$version/$TARGET_OS/$TARGET_ARCH"
  mkdir -p "$stage_dir"
  local tmp_dir
  tmp_dir="$(mktemp -d "$OUT_DIR/.pkg-codex.XXXXXX")"
  cp "$binary_path" "$tmp_dir/codex-crp"
  chmod +x "$tmp_dir/codex-crp" || true
  if [[ "$TARGET_OS" == "macos" ]]; then
    require_cmd strip
    chmod u+w "$tmp_dir/codex-crp" || true
    strip -S -x "$tmp_dir/codex-crp"
    chmod +x "$tmp_dir/codex-crp" || true
  fi
  local archive_path="$stage_dir/codex-crp-$version-$RUST_TARGET.tar.gz"
  create_deterministic_tar_gz "$archive_path" "$tmp_dir" "codex-crp"
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

  local root_entries=()
  local candidate
  for candidate in package.json dist node_modules; do
    if [[ -e "$workspace_dir/$candidate" ]]; then
      root_entries+=("$candidate")
    fi
  done
  local entry_dir="${entrypoint_rel%%/*}"
  if [[ "$entry_dir" != "dist" && "$entry_dir" != "node_modules" && "$entry_dir" != "package.json" && -e "$workspace_dir/$entry_dir" ]]; then
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

provider_override_env_var_name() {
  local provider_id="$1"
  local suffix="$2"
  local normalized
  normalized="$(printf '%s' "$provider_id" | tr '[:lower:]' '[:upper:]' | tr '-' '_')"
  printf 'CTX_PROVIDER_DEPS_%s_%s' "$normalized" "$suffix"
}

resolve_prebuilt_rust_provider_binary() {
  local provider_id="$1"
  local env_var
  env_var="$(provider_override_env_var_name "$provider_id" "BIN")"
  local path_value="${!env_var:-}"
  if [[ -z "$path_value" ]]; then
    return 1
  fi
  if [[ ! -f "$path_value" ]]; then
    echo "error: missing prebuilt binary for $provider_id from $env_var: $path_value" >&2
    exit 1
  fi
  printf '%s\n' "$path_value"
}

require_prebuilt_rust_provider_binary() {
  local provider_id="$1"
  local env_var
  env_var="$(provider_override_env_var_name "$provider_id" "BIN")"
  echo "error: provider-deps staging requires a prebuilt binary for $provider_id via $env_var" >&2
  exit 1
}

resolve_prebuilt_provider_archive() {
  local provider_id="$1"
  local env_var
  env_var="$(provider_override_env_var_name "$provider_id" "ARCHIVE")"
  local path_value="${!env_var:-}"
  if [[ -z "$path_value" ]]; then
    return 1
  fi
  if [[ ! -f "$path_value" ]]; then
    echo "error: missing prebuilt archive for $provider_id from $env_var: $path_value" >&2
    exit 1
  fi
  printf '%s\n' "$path_value"
}

require_prebuilt_provider_archive() {
  local provider_id="$1"
  local env_var
  env_var="$(provider_override_env_var_name "$provider_id" "ARCHIVE")"
  echo "error: provider-deps staging requires a prebuilt archive for $provider_id via $env_var" >&2
  exit 1
}

stage_archive_from_prebuilt_archive() {
  local provider_id="$1"
  local version="$2"
  local prebuilt_archive="$3"
  local archive_name="$4"
  local stage_dir="$OUT_DIR/providers/$provider_id/$version/$TARGET_OS/$TARGET_ARCH"
  mkdir -p "$stage_dir"
  local archive_path="$stage_dir/${archive_name}"
  cp "$prebuilt_archive" "$archive_path"
  printf '%s\n' "$archive_path"
}

build_rust_provider() {
  local provider_id="$1"
  local project_dir="$2"
  local binary_name="$3"
  local version="$4"
  local package_name="${5:-$binary_name}"
  local bin
  bin="$(resolve_prebuilt_rust_provider_binary "$provider_id" || true)"
  if [[ -z "$bin" ]]; then
    if [[ "${CTX_PROVIDER_DEPS_REQUIRE_PREBUILT:-0}" == "1" ]]; then
      require_prebuilt_rust_provider_binary "$provider_id"
    fi
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
      node "$ROOT/core/scripts/run_with_ctx_cache_env.cjs" --mode workspace --cwd "$project_dir" -- cargo build --release --target "$RUST_TARGET" --package "$package_name" --bin "$binary_name"
    ) >&2

    local target_root
    target_root="$(resolve_rust_target_root "$project_dir")"
    bin="$target_root/$RUST_TARGET/release/$binary_name"
  fi
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

  local archive_path
  archive_path="$(resolve_prebuilt_provider_archive "$provider_id" || true)"
  if [[ -n "$archive_path" ]]; then
    archive_path="$(stage_archive_from_prebuilt_archive \
      "$provider_id" \
      "$version" \
      "$archive_path" \
      "${provider_id}-${version}-${TARGET_OS}-${TARGET_ARCH}.tar.gz")"
  else
    if [[ "${CTX_PROVIDER_DEPS_REQUIRE_PREBUILT:-0}" == "1" ]]; then
      require_prebuilt_provider_archive "$provider_id"
    fi
    local workspace_dir
    workspace_dir="$(prepare_node_workspace "$project_dir")"
    chmod +x "$workspace_dir/$entrypoint_rel" || true
    archive_path="$(stage_archive_from_node_project "$provider_id" "$version" "$workspace_dir" "$entrypoint_rel")"
  fi

  local sha
  sha="$(sha256_file "$archive_path")"
  local size_bytes
  size_bytes="$(file_size_bytes "$archive_path")"
  write_entry "$provider_id" "$version" "tar_gz" "$entrypoint_rel" "$archive_path" "$sha" "$size_bytes"
}

build_claude_crp_provider() {
  local provider_id="claude-crp"
  local version="$1"
  local prebuilt_archive
  prebuilt_archive="$(resolve_prebuilt_provider_archive "$provider_id" || true)"
  if [[ -n "$prebuilt_archive" ]]; then
    local staged_archive
    staged_archive="$(stage_archive_from_prebuilt_archive \
      "$provider_id" \
      "$version" \
      "$prebuilt_archive" \
      "${provider_id}-${version}-${TARGET_OS}-${TARGET_ARCH}.tar.gz")"
    local sha
    sha="$(sha256_file "$staged_archive")"
    local size_bytes
    size_bytes="$(file_size_bytes "$staged_archive")"
    write_entry "$provider_id" "$version" "tar_gz" "bin/claude-crp" "$staged_archive" "$sha" "$size_bytes"
    return 0
  fi
  if [[ "${CTX_PROVIDER_DEPS_REQUIRE_PREBUILT:-0}" == "1" ]]; then
    require_prebuilt_provider_archive "$provider_id"
  fi

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

build_codex_crp_provider() {
  local provider_id="codex-crp"
  local version="$1"
  local bin
  bin="$(resolve_prebuilt_rust_provider_binary "$provider_id" || true)"
  if [[ -z "$bin" ]]; then
    if [[ "${CTX_PROVIDER_DEPS_REQUIRE_PREBUILT:-0}" == "1" ]]; then
      require_prebuilt_rust_provider_binary "$provider_id"
    fi
    local rustflags="${RUSTFLAGS:-}"
    if [[ -n "$rustflags" ]]; then
      rustflags="$rustflags -C debuginfo=0"
    else
      rustflags="-C debuginfo=0"
    fi
    require_cmd cargo
    (
      cd "$ROOT/core"
      CARGO_INCREMENTAL=0 \
      CARGO_PROFILE_RELEASE_DEBUG=0 \
      CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO=off \
      RUSTFLAGS="$rustflags" \
      node "$ROOT/core/scripts/run_with_ctx_cache_env.cjs" --mode workspace --cwd "$ROOT/core" -- cargo build --release --target "$RUST_TARGET" --package "codex-crp" --bin "codex-crp"
    ) >&2
    local target_root
    target_root="$(resolve_rust_target_root "$ROOT/core")"
    bin="$target_root/$RUST_TARGET/release/codex-crp"
  fi
  if [[ ! -f "$bin" ]]; then
    echo "error: missing built binary for $provider_id at $bin" >&2
    exit 1
  fi

  local archive_path
  archive_path="$(stage_codex_archive_from_binary "$version" "$bin")"
  local sha
  sha="$(sha256_file "$archive_path")"
  local size_bytes
  size_bytes="$(file_size_bytes "$archive_path")"
  write_entry "$provider_id" "$version" "tar_gz" "codex-crp" "$archive_path" "$sha" "$size_bytes"
}

providers=()
if [[ -n "$PROVIDERS_RAW" ]]; then
  IFS=',' read -r -a providers <<< "$PROVIDERS_RAW"
fi
if [[ -n "${providers[*]-}" ]]; then
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
      auggie|claude-cli|cline|copilot|cursor|gemini|qwen)
        stage_matrix_npm_provider "$provider" "$version"
        ;;
      pi)
        build_node_project_provider "$provider" "$ROOT/harness-adapters/pi-acp" "dist/bin/pi-acp.js" "$version"
        ;;
      kimi|mistral|openhands)
        stage_matrix_python_provider "$provider"
        ;;
      codex-cli|goose|opencode)
        stage_matrix_archive_provider "$provider" "$version"
        ;;
      codex-crp)
        build_codex_crp_provider "$version"
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
fi

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
