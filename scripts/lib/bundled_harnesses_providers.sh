# Provider packaging and runtime manifest emission for bundled harness prep.

skip_runtimes_raw="${CTX_BUNDLE_SKIP_RUNTIMES:-}"
skip_images_raw="${CTX_BUNDLE_SKIP_IMAGES:-}"
only_providers_raw="${CTX_BUNDLE_ONLY_PROVIDERS:-}"
only_providers_raw="${only_providers_raw// /}"
skip_providers_raw="${CTX_BUNDLE_SKIP_PROVIDERS:-}"
skip_providers_raw="${skip_providers_raw// /}"

if [[ -n "$only_providers_raw" ]]; then
  only_providers_raw="$(
    run_python - "$MATRIX_JSON" "$only_providers_raw" "$skip_providers_raw" <<'PY'
import json
import sys
from collections import deque

matrix_path = sys.argv[1]
only_raw = sys.argv[2]
skip_raw = sys.argv[3]

only = [value for value in only_raw.split(",") if value]
skip = {value for value in skip_raw.split(",") if value}
if not only:
    print("")
    raise SystemExit(0)

with open(matrix_path, "r", encoding="utf-8") as fh:
    data = json.load(fh)

providers = {
    str(provider.get("id") or "").strip(): provider
    for provider in data.get("providers", [])
    if str(provider.get("id") or "").strip()
}

ordered = []
seen = set()
queue = deque(only)
while queue:
    provider_id = queue.popleft()
    if not provider_id or provider_id in seen or provider_id in skip:
        continue
    seen.add(provider_id)
    ordered.append(provider_id)
    provider = providers.get(provider_id) or {}
    for dependency in provider.get("provider_dependencies") or []:
        dep_id = str(dependency.get("id") or "").strip()
        if dep_id and dep_id not in seen and dep_id not in skip:
            queue.append(dep_id)

print(",".join(ordered))
PY
  )"
fi

provider_selected_for_bundle() {
  local provider_id="$1"
  if [[ -n "$only_providers_raw" ]]; then
    if [[ ",$only_providers_raw," != *",$provider_id,"* ]]; then
      return 1
    fi
  fi
  if [[ -n "$skip_providers_raw" ]]; then
    if [[ ",$skip_providers_raw," == *",$provider_id,"* ]]; then
      return 1
    fi
  fi
  return 0
}

runtime_need_node="1"
runtime_need_python="1"
if [[ "${CTX_BUNDLE_DEPENDENCY_AWARE_RUNTIMES:-1}" == "1" ]]; then
  read -r runtime_need_node runtime_need_python < <(
    run_python - "$MATRIX_JSON" "$only_providers_raw" "$skip_providers_raw" <<'PY'
import json
import sys

matrix_path = sys.argv[1]
only_raw = sys.argv[2]
skip_raw = sys.argv[3]

only = {v for v in only_raw.split(",") if v}
skip = {v for v in skip_raw.split(",") if v}

def include(provider_id: str) -> bool:
    if only and provider_id not in only:
        return False
    if provider_id in skip:
        return False
    return True

needs_node = False
needs_python = False

with open(matrix_path, "r", encoding="utf-8") as fh:
    data = json.load(fh)

for provider in data.get("providers", []):
    provider_id = provider.get("id", "")
    if not provider_id or not include(provider_id):
        continue
    managed = provider.get("managed_install") or {}
    kind = managed.get("kind")
    if kind == "npm":
        needs_node = True
    elif kind == "archive":
        targets = managed.get("targets") or {}
        for target in targets.values():
            bin_path = ((target or {}).get("bin_path") or "").lower()
            if bin_path.endswith((".js", ".mjs", ".cjs")):
                needs_node = True
                break
    elif kind == "python":
        needs_python = True

print("1" if needs_node else "0", "1" if needs_python else "0")
PY
  )
fi

if ! is_falsy "$LOCAL_ADAPTER_MODE"; then
  if provider_selected_for_bundle "pi"; then
    runtime_need_node="1"
  fi
fi
if provider_selected_for_bundle "claude-crp" || provider_selected_for_bundle "claude-cli"; then
  runtime_need_node="1"
fi
if provider_selected_for_bundle "cline"; then
  runtime_need_node="1"
fi

python_specs_src="$(mktemp /tmp/ctx-bundle-python-specs.XXXXXX)"
: > "$python_specs_src"
if [[ "$runtime_need_python" == "1" ]]; then
  run_python - "$MATRIX_JSON" "$only_providers_raw" "$skip_providers_raw" "$PYTHON_VERSION" "$PYTHON_BUILD_TAG" > "$python_specs_src" <<'PY'
import json
import sys

matrix_path = sys.argv[1]
only_raw = sys.argv[2]
skip_raw = sys.argv[3]
default_version = sys.argv[4]
default_build_tag = sys.argv[5]

only = {value for value in only_raw.split(",") if value}
skip = {value for value in skip_raw.split(",") if value}
seen = set()

def include(provider_id: str) -> bool:
    if only and provider_id not in only:
        return False
    if provider_id in skip:
        return False
    return True

with open(matrix_path, "r", encoding="utf-8") as fh:
    data = json.load(fh)

for provider in data.get("providers", []):
    provider_id = str(provider.get("id") or "").strip()
    if not provider_id or not include(provider_id):
        continue
    managed = provider.get("managed_install") or {}
    if managed.get("kind") != "python":
        continue
    version = str(managed.get("python_version") or default_version).strip()
    build_tag = str(managed.get("python_build_tag") or default_build_tag).strip()
    key = (version, build_tag)
    if key in seen:
        continue
    seen.add(key)
    print(f"{version}\x1f{build_tag}")
PY
fi

if provider_selected_for_bundle "claude-crp"; then
  if [[ ! -d "$CLAUDE_CRP_WORKSPACE" ]]; then
    log "error: claude-crp local-only bundling requires workspace at $CLAUDE_CRP_WORKSPACE"
    exit 5
  fi
  if [[ ! -f "$CLAUDE_CRP_WORKSPACE/dist/runtime.js" ]]; then
    if [[ ! -x "$ROOT/scripts/build_claude_crp.sh" ]]; then
      log "error: missing claude-crp build script at $ROOT/scripts/build_claude_crp.sh"
      exit 5
    fi
    if ! command -v pnpm >/dev/null 2>&1; then
      log "error: bundling claude-crp requires pnpm to build local adapter payload"
      exit 5
    fi
  fi
fi

if provider_selected_for_bundle "claude-crp" || provider_selected_for_bundle "claude-cli"; then
  runtime_need_node="1"
fi

if provider_selected_for_bundle "claude-crp"; then
  if [[ ! -d "$CLAUDE_CRP_WORKSPACE" ]]; then
    log "error: claude-crp local-only bundling requires workspace at $CLAUDE_CRP_WORKSPACE"
    exit 5
  fi
  if [[ ! -f "$CLAUDE_CRP_WORKSPACE/dist/runtime.js" ]]; then
    if [[ ! -x "$ROOT/scripts/build_claude_crp.sh" ]]; then
      log "error: missing claude-crp build script at $ROOT/scripts/build_claude_crp.sh"
      exit 5
    fi
    if ! command -v pnpm >/dev/null 2>&1; then
      log "error: bundling claude-crp requires pnpm to build local adapter payload"
      exit 5
    fi
  fi
fi

if ! is_truthy "$skip_runtimes_raw"; then
  if [[ "$runtime_need_node" == "1" ]]; then
    ensure_node_runtime
  fi
  if [[ "$runtime_need_python" == "1" ]]; then
    while IFS=$'\x1f' read -r python_version python_build_tag; do
      if [[ -z "$python_version" || -z "$python_build_tag" ]]; then
        continue
      fi
      ensure_python_runtime_versioned "$python_version" "$python_build_tag"
    done < "$python_specs_src"
  fi
fi
if ! is_falsy "$INCLUDE_BRIDGE"; then
  require_bridge_binary
fi

if is_truthy "$BUILD_LOCAL_ADAPTERS"; then
  if is_falsy "$LOCAL_ADAPTER_MODE"; then
    log "warn: CTX_BUNDLE_BUILD_LOCAL_ADAPTERS set but CTX_BUNDLE_LOCAL_ADAPTERS=off"
  fi
  build_local_adapters
fi

node_root_rel=""
node_bin_rel=""
npm_cli_rel=""
node_root=""
node_bin=""
npm_cli=""

if ! is_truthy "$skip_runtimes_raw"; then
  if [[ "$runtime_need_node" == "1" ]]; then
    node_root_rel="runtimes/node/${os}/${arch}/node-v${NODE_VERSION}-${node_target}"
    if [[ "$os" == "windows" ]]; then
      node_bin_rel="node.exe"
      npm_cli_rel="node_modules/npm/bin/npm-cli.js"
    else
      node_bin_rel="bin/node"
      npm_cli_rel="lib/node_modules/npm/bin/npm-cli.js"
    fi
    node_root="$bundle_dir/$node_root_rel"
    node_bin="$node_root/$node_bin_rel"
    npm_cli="$node_root/$npm_cli_rel"
  fi
fi

providers_src="$(mktemp /tmp/ctx-bundle-providers.XXXXXX)"
run_python - "$MATRIX_JSON" "$target_key" "$ROOT/core/crates/ctx-http/Cargo.toml" > "$providers_src" <<'PY'
import json
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
target = sys.argv[2]
cargo = Path(sys.argv[3])
sep = "\x1f"

def parse_version_loose(raw: str):
    if not raw:
        return None
    trimmed = raw.strip().lstrip("v")
    if not trimmed:
        return None
    if trimmed.count(".") == 1:
        trimmed = f"{trimmed}.0"
    match = re.match(r"([0-9]+(?:\\.[0-9]+)*)", trimmed)
    if not match:
        return None
    try:
        return tuple(int(part) for part in match.group(1).split("."))
    except ValueError:
        return None

def release_matches_context(release, ctx_version):
    if ctx_version is None:
        return True
    min_v = parse_version_loose(release.get("context_min", ""))
    if min_v and ctx_version < min_v:
        return False
    max_v = parse_version_loose(release.get("context_max", ""))
    if max_v and ctx_version > max_v:
        return False
    return True

def select_latest_release(candidates):
    best = None
    best_v = None
    for release in candidates:
        parsed = parse_version_loose(release.get("version", ""))
        if parsed is None:
            continue
        if best_v is None or parsed > best_v:
            best_v = parsed
            best = release
    if best is not None:
        return best
    return candidates[-1] if candidates else None

ctx_version = None
try:
    for line in cargo.read_text().splitlines():
        if line.strip().startswith("version"):
            _, raw = line.split("=", 1)
            ctx_version = parse_version_loose(raw.strip().strip('"'))
            break
except FileNotFoundError:
    ctx_version = None

data = json.loads(path.read_text())
for provider in data.get("providers", []):
    mi = provider.get("managed_install") or {}
    kind = mi.get("kind")
    if not kind:
        continue
    # If a provider declares releases, respect them: skip managed installs that
    # are not marked supported for this ctx version (e.g. pending/blocked).
    releases = provider.get("releases", [])
    if releases:
        supported = [r for r in releases if r.get("status") == "supported"]
        supported = [r for r in supported if release_matches_context(r, ctx_version)]
        if not supported:
            continue
    args = mi.get("args") or []
    if kind == "archive":
        target_entry = mi.get("targets", {}).get(target)
        if not target_entry:
            continue
        line = sep.join(
            [
                provider.get("id", ""),
                "archive",
                mi.get("version", ""),
                target_entry.get("url", ""),
                target_entry.get("archive", ""),
                target_entry.get("bin_path", ""),
                "",
                "",
                json.dumps(args, separators=(",", ":")),
                "",
                "",
            ]
        )
        print(line)
    elif kind == "npm":
        releases = [
            r for r in provider.get("releases", []) if r.get("status") == "supported"
        ]
        releases = [r for r in releases if release_matches_context(r, ctx_version)]
        release = select_latest_release(releases)
        version = release.get("version", "") if release else ""
        line = sep.join(
            [
                provider.get("id", ""),
                "npm",
                version,
                "",
                "",
                "",
                mi.get("package", ""),
                mi.get("entrypoint", ""),
                json.dumps(args, separators=(",", ":")),
                "",
                "",
            ]
        )
        print(line)
    elif kind == "python":
        line = sep.join(
            [
                provider.get("id", ""),
                "python",
                mi.get("version", ""),
                "",
                "",
                "",
                mi.get("package", ""),
                mi.get("entrypoint", ""),
                json.dumps(args, separators=(",", ":")),
                mi.get("python_version", ""),
                mi.get("python_build_tag", ""),
            ]
        )
        print(line)
PY

provider_archive_dependency_rows() {
  local provider_id="$1"
  run_python - "$MATRIX_JSON" "$provider_id" "$target_key" <<'PY'
import json
import sys

path = sys.argv[1]
provider_id = sys.argv[2]
target = sys.argv[3]
sep = "\x1f"

data = json.loads(open(path, "r", encoding="utf-8").read())
provider = next((entry for entry in data.get("providers", []) if entry.get("id") == provider_id), None)
if provider is None:
    raise SystemExit(0)

missing = []
for dependency in provider.get("dependencies") or []:
    install = dependency.get("install") or {}
    if install.get("kind") != "archive":
        continue
    dep_id = str(dependency.get("id") or "").strip()
    target_entry = (install.get("targets") or {}).get(target)
    if not target_entry:
        missing.append(dep_id or "<unnamed>")
        continue
    print(
        sep.join(
            [
                dep_id,
                str(install.get("version") or "").strip(),
                str(target_entry.get("url") or "").strip(),
                str(target_entry.get("archive") or "").strip(),
                str(target_entry.get("bin_path") or "").strip(),
                str(target_entry.get("sha256") or "").strip(),
            ]
        )
    )

if missing:
    print(
        f"error: provider {provider_id} archive dependency missing target {target}: {', '.join(missing)}",
        file=sys.stderr,
    )
    raise SystemExit(2)
PY
}

provider_bundle_version_marker() {
  local provider_id="$1"
  local provider_version="$2"
  run_python - "$MATRIX_JSON" "$provider_id" "$provider_version" "$target_key" <<'PY'
import hashlib
import json
import sys

path = sys.argv[1]
provider_id = sys.argv[2]
provider_version = sys.argv[3]
target = sys.argv[4]

data = json.loads(open(path, "r", encoding="utf-8").read())
provider = next((entry for entry in data.get("providers", []) if entry.get("id") == provider_id), None)
deps = []
if provider is not None:
    for dependency in provider.get("dependencies") or []:
        install = dependency.get("install") or {}
        if install.get("kind") != "archive":
            continue
        dep_id = str(dependency.get("id") or "").strip()
        target_entry = (install.get("targets") or {}).get(target) or {}
        deps.append(
            {
                "id": dep_id,
                "version": str(install.get("version") or "").strip(),
                "url": str(target_entry.get("url") or "").strip(),
                "archive": str(target_entry.get("archive") or "").strip(),
                "bin_path": str(target_entry.get("bin_path") or "").strip(),
                "sha256": str(target_entry.get("sha256") or "").strip(),
            }
        )

payload = {
    "provider_id": provider_id,
    "provider_version": provider_version,
    "target": target,
    "dependencies": deps,
}
encoded = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
print(hashlib.sha256(encoded).hexdigest())
PY
}

bundle_archive_dependency_into_provider_root() {
  local provider_id="$1"
  local dependency_id="$2"
  local dependency_url="$3"
  local dependency_archive="$4"
  local dependency_bin_path="$5"
  local provider_root="$6"

  if [[ -z "$dependency_url" || -z "$dependency_archive" || -z "$dependency_bin_path" ]]; then
    log "error: incomplete archive dependency metadata for $provider_id/$dependency_id"
    exit 5
  fi

  local tmp_file
  tmp_file="$(mktemp -p "$provider_root" "${dependency_id}.XXXXXX")"
  fetch_file "$dependency_url" "$tmp_file"
  case "$dependency_archive" in
    none)
      local dest="$provider_root/$dependency_bin_path"
      mkdir -p "$(dirname "$dest")"
      mv "$tmp_file" "$dest"
      ;;
    tar_gz)
      require_cmd tar
      tar -xzf "$tmp_file" -C "$provider_root"
      rm -f "$tmp_file"
      ;;
    tar_bz2)
      require_cmd tar
      tar -xjf "$tmp_file" -C "$provider_root"
      rm -f "$tmp_file"
      ;;
    zip)
      require_cmd unzip
      unzip -q "$tmp_file" -d "$provider_root"
      rm -f "$tmp_file"
      ;;
    dmg)
      if [[ "$os" != "macos" ]]; then
        log "error: archive dependency '$dependency_id' for $provider_id requires macOS host tooling"
        exit 5
      fi
      require_cmd hdiutil
      local dmg_mount_dir
      dmg_mount_dir="$(mktemp -d "/tmp/ctx-dmg-${provider_id}-${dependency_id}.XXXXXX")"
      if ! hdiutil attach -nobrowse -readonly -mountpoint "$dmg_mount_dir" "$tmp_file" >/dev/null; then
        rm -rf "$dmg_mount_dir" "$tmp_file"
        log "error: failed to mount dmg for dependency $dependency_id ($provider_id)"
        exit 5
      fi
      if ! copy_dmg_payload_without_external_symlinks "$dmg_mount_dir" "$provider_root"; then
        hdiutil detach "$dmg_mount_dir" -force >/dev/null 2>&1 || true
        rm -rf "$dmg_mount_dir" "$tmp_file"
        log "error: failed to copy dmg payload for dependency $dependency_id ($provider_id)"
        exit 5
      fi
      hdiutil detach "$dmg_mount_dir" -force >/dev/null 2>&1 || true
      rm -rf "$dmg_mount_dir" "$tmp_file"
      ;;
    *)
      log "error: unsupported archive dependency type '$dependency_archive' for $provider_id/$dependency_id"
      exit 5
      ;;
  esac

  local dependency_command_path="$provider_root/$dependency_bin_path"
  if [[ ! -f "$dependency_command_path" ]]; then
    dependency_command_path="$(resolve_unique_path "$provider_root" "$dependency_bin_path")"
  fi
  if [[ -z "$dependency_command_path" || ! -f "$dependency_command_path" ]]; then
    log "error: bundled dependency binary missing for $provider_id/$dependency_id: $dependency_bin_path"
    exit 5
  fi
  if [[ "$os" != "windows" ]]; then
    chmod +x "$dependency_command_path" || true
  fi
  maybe_adhoc_codesign_macos_binary "$dependency_command_path"
}

local_providers_src="$(mktemp /tmp/ctx-bundle-local-providers.XXXXXX)"
local_ids=()

add_local_provider() {
  local provider_id="$1"
  local kind="$2"
  local version="$3"
  local source_path="$4"
  local bin_path="$5"
  local args_json="${6:-[]}"
  if [[ -s "$local_providers_src" ]]; then
    local filtered
    filtered="$(mktemp /tmp/ctx-bundle-local-providers-filtered.XXXXXX)"
    awk -F $'\x1f' -v id="$provider_id" '$1 != id' "$local_providers_src" > "$filtered"
    mv "$filtered" "$local_providers_src"
  fi
  local sep=$'\x1f'
  printf '%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s\n' \
    "$provider_id" "$sep" "$kind" "$sep" "$version" "$sep" \
    "$source_path" "$sep" "" "$sep" "$bin_path" "$sep" "" "$sep" "" "$sep" \
    "$args_json" "$sep" "" "$sep" "" >> "$local_providers_src"
  local found=0
  local existing
  for existing in "${local_ids[@]-}"; do
    if [[ "$existing" == "$provider_id" ]]; then
      found=1
      break
    fi
  done
  if [[ "$found" == "0" ]]; then
    local_ids+=("$provider_id")
  fi
}

if ! is_falsy "$INCLUDE_BRIDGE"; then
  bridge_src="$(local_bridge_binary_path)"
  add_local_provider "acp-crp-bridge" "local-bin" "local" "$bridge_src" "$(basename "$bridge_src")" "[]"
fi

should_build_codex_crp() {
  if ! provider_selected_for_bundle "codex"; then
    return 1
  fi
  if is_truthy "$CODEX_CRP_BUILD_MODE"; then
    return 0
  fi
  if is_falsy "$CODEX_CRP_BUILD_MODE"; then
    return 1
  fi
  log "error: invalid CTX_BUNDLE_BUILD_CODEX_CRP='${CODEX_CRP_BUILD_MODE}' (expected 0/1)"
  exit 5
}

local_codex_crp_binary_path() {
  if [[ ! -d "$CODEX_CRP_WORKSPACE" ]]; then
    return 1
  fi
  local profile="${CTX_BUNDLE_CODEX_CRP_PROFILE:-release}"
  local target_dir="${CTX_BUNDLE_CODEX_CRP_TARGET_DIR:-$bundle_build_dir/codex-crp/${os}/${arch}}"

	local profile_args=()
	if [[ "$profile" == "release" ]]; then
	  profile_args+=(--release)
	elif [[ "$profile" != "debug" ]]; then
	  log "error: invalid CTX_BUNDLE_CODEX_CRP_PROFILE: $profile (expected debug|release)"
	  exit 5
	fi

	# codex-crp release defaults are intentionally very heavy upstream; use a lighter optimized
	# profile so local bundling is practical in CI/dev and consistent with container builds below.
	local -a cargo_profile_env=()
	if [[ "$profile" == "release" ]]; then
	  cargo_profile_env+=(
	    "CARGO_PROFILE_RELEASE_LTO=false"
	    "CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16"
	    "CARGO_PROFILE_RELEASE_OPT_LEVEL=2"
	  )
	fi

	build_codex_crp_in_container() {
	  if ! ensure_docker_ready_for_builds "0" "codex-crp container build"; then
	    if ! command -v cargo-zigbuild >/dev/null 2>&1 || ! command -v zig >/dev/null 2>&1; then
	      log "error: building codex-crp for ${os}/${arch} requires either Docker or cargo-zigbuild+zig."
	      log "       Ensure Docker Desktop is running, or install cargo-zigbuild and zig, then retry."
	      exit 5
	    fi
	    mkdir -p "$target_dir"
	    (
	      cd "$CODEX_CRP_WORKSPACE"
	      env CARGO_TARGET_DIR="$target_dir" "${cargo_profile_env[@]}" cargo zigbuild --manifest-path "$CODEX_CRP_WORKSPACE/Cargo.toml" -p codex-crp --target "$rust_target" "${profile_args[@]}"
	    )
	    return
	  fi

	  mkdir -p "$target_dir"

	  local platform="linux/arm64"
	  if [[ "$arch" == "x86_64" ]]; then
	    platform="linux/amd64"
	  fi

	  local image="${CTX_BUNDLE_RUST_IMAGE:-rust:1.88-bookworm}"

	  local -a run_args
	  run_args=(run --rm --platform "$platform" -v "$CODEX_CRP_WORKSPACE:/work:rw" -v "$target_dir:/target:rw" -w /work -e CARGO_TARGET_DIR=/target)

	  # Avoid `bash -l` here: login shells can reset PATH and drop Cargo.
	  #
	  # Also: `codex-crp` upstream ships with a very heavy release profile (fat LTO, 1 codegen unit),
	  # which can OOM on typical Docker Desktop configs. Override to a lighter release build: still
	  # optimized, but much less memory hungry.
	  docker "${run_args[@]}" "$image" bash -c "set -euo pipefail; export PATH=\"/usr/local/cargo/bin:\$PATH\"; rustup toolchain install stable --profile minimal --no-self-update >/dev/null 2>&1 || true; rustup target add --toolchain stable '$rust_target' >/dev/null 2>&1 || true; ${cargo_profile_env[*]} cargo +stable build --manifest-path /work/Cargo.toml -p codex-crp --target '$rust_target' ${profile_args[*]}"
	}

	if [[ "$os" == "linux" && "$host_os" != "linux" ]]; then
	  build_codex_crp_in_container
	else
	  require_cmd cargo
	  (
	    cd "$CODEX_CRP_WORKSPACE"
	    env CARGO_TARGET_DIR="$target_dir" "${cargo_profile_env[@]}" node "$ROOT/core/scripts/run_with_ctx_cache_env.cjs" --mode workspace --cwd "$CODEX_CRP_WORKSPACE" -- cargo build --manifest-path "$CODEX_CRP_WORKSPACE/Cargo.toml" -p codex-crp --target "$rust_target" "${profile_args[@]}"
	  )
	fi

	local bin="$target_dir/$rust_target/$profile/codex-crp$BIN_EXT"
	if [[ ! -f "$bin" ]]; then
	  log "error: codex-crp binary not found at $bin"
	  exit 5
	fi
	printf '%s' "$bin"
}

should_bundle_local_claude_crp() {
  if ! provider_selected_for_bundle "claude-crp"; then
    return 1
  fi
  if [[ ! -d "$CLAUDE_CRP_WORKSPACE" ]]; then
    log "error: claude-crp local-only bundling requires workspace at $CLAUDE_CRP_WORKSPACE"
    exit 5
  fi
  return 0
}

ensure_local_claude_crp_dist() {
  local dist_entry="$CLAUDE_CRP_WORKSPACE/dist/runtime.js"
  if [[ -f "$dist_entry" ]]; then
    return 0
  fi
  if ! should_bundle_local_claude_crp; then
    return 1
  fi
  if [[ ! -x "$ROOT/scripts/build_claude_crp.sh" ]]; then
    log "error: missing claude-crp build script at $ROOT/scripts/build_claude_crp.sh"
    return 1
  fi
  if ! command -v pnpm >/dev/null 2>&1; then
    log "error: bundling claude-crp requires pnpm to build local adapter payload"
    return 1
  fi
  if ! "$ROOT/scripts/build_claude_crp.sh"; then
    log "error: failed to build local claude-crp bundle payload"
    return 1
  fi
  if [[ ! -f "$dist_entry" ]]; then
    log "error: claude-crp build completed without dist entrypoint at $dist_entry"
    return 1
  fi
  return 0
}

if should_build_codex_crp; then
  codex_crp_version="$(get_matrix_version "codex")"
  if [[ -z "$codex_crp_version" ]]; then
    codex_crp_version="local"
  fi
  codex_crp_bin="$(local_codex_crp_binary_path || true)"
  if [[ -n "$codex_crp_bin" && -f "$codex_crp_bin" ]]; then
    add_local_provider "codex" "local-bin" "$codex_crp_version" "$codex_crp_bin" "codex-crp$BIN_EXT" "[\"codex-cli\"]"
  else
    log "error: codex-crp build requested but source not available at $CODEX_CRP_WORKSPACE"
    exit 5
  fi
fi

if should_bundle_local_claude_crp; then
  claude_crp_version="$(get_matrix_version "claude-crp")"
  if [[ -z "$claude_crp_version" ]]; then
    claude_crp_version="$(package_json_version "$CLAUDE_CRP_WORKSPACE/package.json")"
  fi
  if [[ -z "$claude_crp_version" ]]; then
    claude_crp_version="local"
  fi

  if ! ensure_local_claude_crp_dist; then
    log "error: failed to prepare local claude-crp bundle payload at $CLAUDE_CRP_WORKSPACE/dist/runtime.js"
    exit 5
  fi
  if [[ ! -f "$CLAUDE_CRP_WORKSPACE/bin/claude-crp" ]]; then
    log "error: local claude-crp entrypoint missing at $CLAUDE_CRP_WORKSPACE/bin/claude-crp"
    exit 5
  fi
  add_local_provider "claude-crp" "local-node" "$claude_crp_version" "$CLAUDE_CRP_WORKSPACE" "bin/claude-crp" "[]"
fi

if ! is_falsy "$LOCAL_ADAPTER_MODE"; then
  local_adapter_required=0
  if is_truthy "$LOCAL_ADAPTER_MODE"; then
    local_adapter_required=1
  fi

  adapter_version_override="${CTX_BUNDLE_ADAPTER_VERSION:-}"
  for id in pi droid; do
    if ! provider_selected_for_bundle "$id"; then
      continue
    fi
    version="$adapter_version_override"
    if [[ -z "$version" ]]; then
      version="$(get_matrix_version "$id")"
    fi
    if [[ -z "$version" ]]; then
      version="local"
    fi

    if [[ "$id" == "pi" ]]; then
      src="$(local_adapter_node_entrypoint "$id")"
      if [[ ! -f "$src" ]]; then
        dir="$(local_adapter_dir "$id")"
        if [[ -n "$dir" && -d "$LOCAL_ADAPTERS_DIR/$dir" ]]; then
          if command -v pnpm >/dev/null 2>&1; then
            (cd "$LOCAL_ADAPTERS_DIR/$dir" && pnpm install --ignore-scripts)
            (cd "$LOCAL_ADAPTERS_DIR/$dir" && pnpm run build)
          else
            require_cmd npm
            (cd "$LOCAL_ADAPTERS_DIR/$dir" && npm install --ignore-scripts)
            (cd "$LOCAL_ADAPTERS_DIR/$dir" && npm run build)
          fi
          src="$(local_adapter_node_entrypoint "$id")"
        fi
      fi
      if [[ -f "$src" ]]; then
        adapter_root="$(local_adapter_root_path "$id" || true)"
        if [[ -n "$adapter_root" && "$src" == "$adapter_root/"* ]]; then
          entrypoint_rel="${src#"$adapter_root/"}"
          add_local_provider "$id" "local-node" "$version" "$adapter_root" "$entrypoint_rel" "[]"
        else
          add_local_provider "$id" "local-node" "$version" "$src" "$(basename "$src")" "[]"
        fi
      elif [[ "$local_adapter_required" == "1" ]]; then
        log "error: missing local-node adapter entrypoint for $id at $src"
        exit 5
      fi
      continue
    fi

    src="$(local_adapter_binary_path "$id" || true)"
    if [[ ! -f "$src" ]]; then
      dir="$(local_adapter_dir "$id")"
      if [[ -n "$dir" && -d "$LOCAL_ADAPTERS_DIR/$dir" ]]; then
        require_cmd cargo
        node "$ROOT/core/scripts/run_with_ctx_cache_env.cjs" --mode workspace --cwd "$LOCAL_ADAPTERS_DIR/$dir" -- cargo build --release --target "$rust_target"
        src="$(local_adapter_binary_path "$id" || true)"
      fi
    fi
    if [[ -f "$src" ]]; then
      add_local_provider "$id" "local-bin" "$version" "$src" "$(basename "$src")" "[]"
    elif [[ "$local_adapter_required" == "1" ]]; then
      log "error: missing local adapter binary for $id at $src"
      exit 5
    fi
  done
fi

if [[ ${#local_ids[@]} -gt 0 ]]; then
  ids_csv="$(IFS=,; echo "${local_ids[*]}")"
  run_python - "$providers_src" "$ids_csv" <<'PY'
import sys

path = sys.argv[1]
ids = {v for v in sys.argv[2].split(",") if v}
sep = "\x1f"

lines = []
with open(path, "r", encoding="utf-8") as fh:
    for line in fh:
        if not line.strip():
            continue
        provider_id = line.split(sep, 1)[0]
        if provider_id in ids:
            continue
        lines.append(line)

with open(path, "w", encoding="utf-8") as fh:
    fh.writelines(lines)
PY
  cat "$local_providers_src" >> "$providers_src"
fi

providers_out="$(mktemp /tmp/ctx-bundle-providers-out.XXXXXX)"

while IFS=$'\x1f' read -r provider_id kind version url archive bin_path package entrypoint args_json python_version python_build_tag; do
  if [[ -z "$provider_id" || -z "$kind" ]]; then
    continue
  fi
  include_provider=0
  if provider_selected_for_bundle "$provider_id"; then
    include_provider=1
  elif [[ "$provider_id" == "claude-cli" ]] && provider_selected_for_bundle "claude-crp"; then
    include_provider=1
  fi
  if [[ "$include_provider" != "1" ]]; then
    continue
  fi

  provider_root="$bundle_dir/providers/${provider_id}/${os}/${arch}"
  version_marker="$provider_root/.version"

  case "$kind" in
    local-bin)
      if [[ -z "$url" ]]; then
        log "error: missing local adapter path for $provider_id"
        exit 5
      fi
      if [[ -z "$bin_path" ]]; then
        bin_path="$(basename "$url")"
      fi
      mkdir -p "$provider_root"
      dest="$provider_root/$bin_path"
      mkdir -p "$(dirname "$dest")"
      rm -f "$dest"
      cp "$url" "$dest"
      if [[ "$os" != "windows" ]]; then
        chmod +x "$dest" || true
      fi
      maybe_adhoc_codesign_macos_binary "$dest"
      echo "$version" > "$version_marker"
      command_path="$dest"
      ;;
    local-node)
      if is_truthy "$skip_runtimes_raw"; then
        log "error: cannot bundle local-node provider $provider_id when CTX_BUNDLE_SKIP_RUNTIMES=1"
        exit 5
      fi
      if [[ -z "$url" ]]; then
        log "error: missing local adapter entrypoint for $provider_id"
        exit 5
      fi
      if [[ -z "$bin_path" ]]; then
        bin_path="$(basename "$url")"
      fi

      if [[ -d "$url" ]]; then
        src_entrypoint="$url/$bin_path"
        if [[ ! -f "$src_entrypoint" ]]; then
          log "error: local-node entrypoint missing for $provider_id: $src_entrypoint"
          exit 5
        fi
        copy_local_node_adapter_payload "$url" "$provider_root"
        if [[ -f "$provider_root/package.json" ]]; then
          npm_install_bundle "$provider_root" "" "project"
        fi
        prune_provider_node_payload "$provider_id" "$provider_root"
      else
        mkdir -p "$provider_root"
        dest="$provider_root/$bin_path"
        mkdir -p "$(dirname "$dest")"
        rm -f "$dest"
        cp "$url" "$dest"
      fi

      dest="$provider_root/$bin_path"
      if [[ ! -f "$dest" ]]; then
        log "error: bundled local-node entrypoint missing for $provider_id: $dest"
        exit 5
      fi
      if [[ -z "$node_bin" || ! -f "$node_bin" ]]; then
        log "error: local-node provider $provider_id requires bundled node runtime"
        exit 5
      fi
      echo "$version" > "$version_marker"
      command_path="$node_bin"
      entrypoint_rel="${dest#"$bundle_dir/"}"
      args_json="$(PROVIDER_ENTRYPOINT="$entrypoint_rel" PROVIDER_ARGS_JSON="$args_json" run_python - <<'PY'
import json
import os

args = [os.environ["PROVIDER_ENTRYPOINT"]]
extra = json.loads(os.environ["PROVIDER_ARGS_JSON"] or "[]")
args.extend(extra)
print(json.dumps(args, separators=(",", ":")))
PY
)"
      ;;
    archive)
      if [[ -z "$version" || -z "$url" ]]; then
        continue
      fi
      bundle_version_marker="$(provider_bundle_version_marker "$provider_id" "$version")"
      if [[ -f "$version_marker" ]]; then
        if [[ "$(cat "$version_marker" 2>/dev/null || true)" != "$bundle_version_marker" ]]; then
          rm -rf "$provider_root"
        fi
      fi

      if [[ ! -d "$provider_root" ]]; then
        mkdir -p "$provider_root"
        tmp_file="$(mktemp -p "$provider_root" "${provider_id}.XXXXXX")"
        fetch_file "$url" "$tmp_file"
        case "$archive" in
          none)
            dest="$provider_root/$bin_path"
            mkdir -p "$(dirname "$dest")"
            mv "$tmp_file" "$dest"
            ;;
          tar_gz)
            require_cmd tar
            tar -xzf "$tmp_file" -C "$provider_root"
            rm -f "$tmp_file"
            ;;
          tar_bz2)
            require_cmd tar
            tar -xjf "$tmp_file" -C "$provider_root"
            rm -f "$tmp_file"
            ;;
          zip)
            require_cmd unzip
            unzip -q "$tmp_file" -d "$provider_root"
            rm -f "$tmp_file"
            ;;
          dmg)
            if [[ "$os" != "macos" ]]; then
              log "error: archive type 'dmg' for $provider_id requires macOS host tooling"
              exit 5
            fi
            require_cmd hdiutil
            dmg_mount_dir="$(mktemp -d "/tmp/ctx-dmg-${provider_id}.XXXXXX")"
            if ! hdiutil attach -nobrowse -readonly -mountpoint "$dmg_mount_dir" "$tmp_file" >/dev/null; then
              rm -rf "$dmg_mount_dir" "$tmp_file"
              log "error: failed to mount dmg for $provider_id"
              exit 5
            fi
            if ! copy_dmg_payload_without_external_symlinks "$dmg_mount_dir" "$provider_root"; then
              hdiutil detach "$dmg_mount_dir" -force >/dev/null 2>&1 || true
              rm -rf "$dmg_mount_dir" "$tmp_file"
              log "error: failed to copy dmg payload for $provider_id"
              exit 5
            fi
            hdiutil detach "$dmg_mount_dir" -force >/dev/null 2>&1 || true
            rm -rf "$dmg_mount_dir" "$tmp_file"
            ;;
          *)
            log "error: unsupported archive type '$archive' for $provider_id"
            exit 5
            ;;
        esac
        while IFS=$'\x1f' read -r dependency_id dependency_version dependency_url dependency_archive dependency_bin_path dependency_sha256; do
          if [[ -z "$dependency_id" ]]; then
            continue
          fi
          bundle_archive_dependency_into_provider_root \
            "$provider_id" \
            "$dependency_id" \
            "$dependency_url" \
            "$dependency_archive" \
            "$dependency_bin_path" \
            "$provider_root"
        done < <(provider_archive_dependency_rows "$provider_id")
        echo "$bundle_version_marker" > "$version_marker"
      fi
      command_path="$provider_root/$bin_path"
      if [[ ! -f "$command_path" ]]; then
        command_path="$(resolve_unique_path "$provider_root" "$bin_path")"
      fi
      if [[ "$os" != "windows" ]]; then
        chmod +x "$command_path" || true
      fi
      maybe_adhoc_codesign_macos_binary "$command_path"
      ;;
    npm)
      if is_truthy "$skip_runtimes_raw"; then
        log "error: cannot bundle npm provider $provider_id when CTX_BUNDLE_SKIP_RUNTIMES=1"
        exit 5
      fi
      if [[ -z "$version" || -z "$package" || -z "$entrypoint" ]]; then
        log "error: missing npm metadata for $provider_id"
        exit 5
      fi

      entrypoint_path="$provider_root/$entrypoint"
      if [[ -f "$version_marker" ]]; then
        if [[ "$(cat "$version_marker" 2>/dev/null || true)" != "$version" ]]; then
          rm -rf "$provider_root"
        elif [[ ! -f "$entrypoint_path" ]]; then
          resolved_entrypoint="$(resolve_npm_entrypoint "$provider_root" "$package" "$entrypoint" || true)"
          if [[ -n "$resolved_entrypoint" && -f "$resolved_entrypoint" ]]; then
            entrypoint_path="$resolved_entrypoint"
          else
            rm -rf "$provider_root"
          fi
        fi
      fi

      if [[ ! -d "$provider_root" ]]; then
        mkdir -p "$provider_root"
        npm_install_bundle "$provider_root" "${package}@${version}"
        entrypoint_path="$provider_root/$entrypoint"
        if [[ ! -f "$entrypoint_path" ]]; then
          entrypoint_path="$(resolve_npm_entrypoint "$provider_root" "$package" "$entrypoint" || true)"
        fi
        if [[ -z "$entrypoint_path" || ! -f "$entrypoint_path" ]]; then
          log "error: npm entrypoint missing for $provider_id: $entrypoint_path"
          exit 5
        fi
        echo "$version" > "$version_marker"
      fi
      prune_provider_node_payload "$provider_id" "$provider_root"
      if [[ -z "$node_bin" || ! -f "$node_bin" ]]; then
        log "error: npm provider $provider_id requires bundled node runtime"
        exit 5
      fi

      command_path="$node_bin"
      entrypoint_rel="${entrypoint_path#"$bundle_dir/"}"
      args_json="$(PROVIDER_ENTRYPOINT="$entrypoint_rel" PROVIDER_ARGS_JSON="$args_json" run_python - <<'PY'
import json
import os

args = [os.environ["PROVIDER_ENTRYPOINT"]]
extra = json.loads(os.environ["PROVIDER_ARGS_JSON"] or "[]")
args.extend(extra)
print(json.dumps(args, separators=(",", ":")))
PY
)"
      ;;
    python)
      if is_truthy "$skip_runtimes_raw"; then
        log "error: cannot bundle python provider $provider_id when CTX_BUNDLE_SKIP_RUNTIMES=1"
        exit 5
      fi
      if [[ -z "$version" || -z "$package" || -z "$entrypoint" ]]; then
        log "error: missing python metadata for $provider_id"
        exit 5
      fi

      venv_dir="$provider_root/venv"
      entrypoint_path="$(venv_exe "$venv_dir" "$entrypoint")"
      provider_python_version="${python_version:-$PYTHON_VERSION}"
      provider_python_build_tag="${python_build_tag:-$PYTHON_BUILD_TAG}"
      provider_python_root_rel="$(python_runtime_root_rel "$provider_python_version" "$provider_python_build_tag")"
      provider_python_bin_rel="$(python_runtime_bin_rel "$provider_python_root_rel")"
      provider_python_bin="$bundle_dir/$provider_python_root_rel/$provider_python_bin_rel"
      bundle_version_marker="${version}|${provider_python_version}|${provider_python_build_tag}"
      if [[ -f "$version_marker" ]]; then
        if [[ "$(cat "$version_marker" 2>/dev/null || true)" != "$bundle_version_marker" || ! -f "$entrypoint_path" ]]; then
          rm -rf "$provider_root"
        fi
      fi

      if [[ ! -d "$provider_root" ]]; then
        mkdir -p "$provider_root"
        if [[ -z "$provider_python_bin" || ! -f "$provider_python_bin" ]]; then
          log "error: python provider $provider_id requires bundled python runtime"
          exit 5
        fi
        "$provider_python_bin" -m venv "$venv_dir"
        venv_python="$(venv_exe "$venv_dir" "python")"
        ensure_venv_pip "$venv_python"

        package_spec="$package"
        if [[ "$package" != http://* && "$package" != https://* ]]; then
          package_spec="${package}==${version}"
        fi

        PIP_DISABLE_PIP_VERSION_CHECK=1 \
        PIP_NO_INPUT=1 \
        "$venv_python" -m pip install --disable-pip-version-check --no-input "$package_spec"

        entrypoint_path="$(venv_exe "$venv_dir" "$entrypoint")"
        if [[ ! -f "$entrypoint_path" ]]; then
          log "error: python entrypoint missing for $provider_id: $entrypoint_path"
          exit 5
        fi
        echo "$bundle_version_marker" > "$version_marker"
      fi

      command_path="$entrypoint_path"
      ;;
    *)
      continue
      ;;
  esac

  if [[ "$command_path" != "$bundle_dir"/* ]]; then
    log "error: resolved command path outside bundle dir: $command_path"
    exit 6
  fi

  rel_command="${command_path#"$bundle_dir/"}"
  sha256="$(sha256_file "$command_path")"
  protocol="acp"
  if [[ "$provider_id" == *"-crp" ]]; then
    protocol="crp"
  fi

  PROVIDER_ID="$provider_id" \
  PROVIDER_VERSION="$version" \
  PROVIDER_PROTOCOL="$protocol" \
  PROVIDER_OS="$os" \
  PROVIDER_ARCH="$arch" \
  PROVIDER_SHA256="$sha256" \
  PROVIDER_COMMAND="$rel_command" \
  PROVIDER_ARGS_JSON="$args_json" \
  run_python - <<'PY' >> "$providers_out"
import json
import os

entry = {
    "id": os.environ["PROVIDER_ID"],
    "protocol": os.environ["PROVIDER_PROTOCOL"],
    "version": os.environ["PROVIDER_VERSION"],
    "os": os.environ["PROVIDER_OS"],
    "arch": os.environ["PROVIDER_ARCH"],
    "sha256": os.environ["PROVIDER_SHA256"],
    "command": os.environ["PROVIDER_COMMAND"],
    "args": json.loads(os.environ["PROVIDER_ARGS_JSON"] or "[]"),
}
print(json.dumps(entry, separators=(",", ":")))
PY

  unset PROVIDER_ID PROVIDER_VERSION PROVIDER_PROTOCOL PROVIDER_OS PROVIDER_ARCH PROVIDER_SHA256 PROVIDER_COMMAND PROVIDER_ARGS_JSON

done < "$providers_src"

runtimes_out="$(mktemp /tmp/ctx-bundle-runtimes-out.XXXXXX)"
images_out="$(mktemp /tmp/ctx-bundle-images-out.XXXXXX)"

if ! is_truthy "$skip_runtimes_raw"; then
  if [[ "$runtime_need_node" == "1" ]]; then
    node_sha="$(sha256_file "$node_bin")"
    NODE_VERSION_ENV="$NODE_VERSION" \
    NODE_OS_ENV="$os" \
    NODE_ARCH_ENV="$arch" \
    NODE_SHA_ENV="$node_sha" \
    NODE_ROOT_REL_ENV="$node_root_rel" \
    NODE_BIN_REL_ENV="$node_bin_rel" \
    NODE_NPM_REL_ENV="$npm_cli_rel" \
    run_python - <<'PY' >> "$runtimes_out"
import json
import os

entry = {
    "id": "node",
    "version": os.environ["NODE_VERSION_ENV"],
    "os": os.environ["NODE_OS_ENV"],
    "arch": os.environ["NODE_ARCH_ENV"],
    "sha256": os.environ["NODE_SHA_ENV"],
    "root": os.environ["NODE_ROOT_REL_ENV"],
    "bin": os.environ["NODE_BIN_REL_ENV"],
    "npm_cli": os.environ["NODE_NPM_REL_ENV"],
}
print(json.dumps(entry, separators=(",", ":")))
PY
  fi

  if [[ "$runtime_need_python" == "1" ]]; then
    while IFS=$'\x1f' read -r python_version python_build_tag; do
      if [[ -z "$python_version" || -z "$python_build_tag" ]]; then
        continue
      fi
      python_root_rel="$(python_runtime_root_rel "$python_version" "$python_build_tag")"
      python_bin_rel="$(python_runtime_bin_rel "$python_root_rel")"
      python_bin="$bundle_dir/$python_root_rel/$python_bin_rel"
      python_sha="$(sha256_file "$python_bin")"
      PYTHON_VERSION_ENV="$python_version" \
      PYTHON_OS_ENV="$os" \
      PYTHON_ARCH_ENV="$arch" \
      PYTHON_SHA_ENV="$python_sha" \
      PYTHON_ROOT_REL_ENV="$python_root_rel" \
      PYTHON_BIN_REL_ENV="$python_bin_rel" \
      run_python - <<'PY' >> "$runtimes_out"
import json
import os

entry = {
    "id": "python",
    "version": os.environ["PYTHON_VERSION_ENV"],
    "os": os.environ["PYTHON_OS_ENV"],
    "arch": os.environ["PYTHON_ARCH_ENV"],
    "sha256": os.environ["PYTHON_SHA_ENV"],
    "root": os.environ["PYTHON_ROOT_REL_ENV"],
    "bin": os.environ["PYTHON_BIN_REL_ENV"],
}
print(json.dumps(entry, separators=(",", ":")))
PY
    done < "$python_specs_src"
  fi

fi
