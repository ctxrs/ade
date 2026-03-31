#!/usr/bin/env bash
set -euo pipefail

suite="${1:-}"
if [[ -z "${suite}" ]]; then
  echo "usage: $0 {e2e|runner|tokens|endpoint-ui|provider-api-auth|provider-browser-auth|write-file|linux-arm-critical|linux-arm-nightly}" >&2
  exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"
http_tests_dir="${repo_root}/crates/ctx-http/tests"
providers_tests_dir="${repo_root}/crates/ctx-providers/tests"
bundle_script="${repo_root}/../scripts/ensure_bundled_harnesses.sh"
preflight_script="${repo_root}/scripts/desktop_e2e_preflight.cjs"
volatile_root="${CTX_VOLATILE_ROOT:-${HOME}/.ctx/volatile}"
volatile_targets_dir="${CTX_VOLATILE_TARGETS_DIR:-${volatile_root}/targets}"
volatile_artifacts_dir="${CTX_VOLATILE_ARTIFACTS_DIR:-${volatile_root}/artifacts}"
volatile_tmp_dir="${CTX_VOLATILE_TMPDIR:-${volatile_root}/tmp}"
default_cargo_target_dir="${CTX_E2E_CARGO_TARGET_DIR:-${volatile_targets_dir}/ctx-e2e/${suite}}"
default_e2e_tmp_dir="${CTX_E2E_TMPDIR:-${volatile_tmp_dir}/ctx-e2e-${suite}-${$}}"

mkdir -p \
  "${default_e2e_tmp_dir}" \
  "${default_cargo_target_dir}" \
  "${default_cargo_target_dir}/debug" \
  "${default_cargo_target_dir}/debug/deps" \
  "${default_cargo_target_dir}/debug/.fingerprint" \
  "${default_cargo_target_dir}/release" \
  "${default_cargo_target_dir}/release/deps" \
  "${default_cargo_target_dir}/release/.fingerprint"

export CTX_E2E_CARGO_TARGET_DIR="${default_cargo_target_dir}"
export CARGO_TARGET_DIR="${default_cargo_target_dir}"
export CTX_E2E_TMPDIR="${default_e2e_tmp_dir}"
export TMPDIR="${default_e2e_tmp_dir}"
export TMP="${default_e2e_tmp_dir}"
export TEMP="${default_e2e_tmp_dir}"

run_preflight() {
  local suite_id="$1"
  shift || true
  local -a cmd=(node "${preflight_script}" --suite "${suite_id}")
  if [[ "${CTX_DESKTOP_E2E_PREFLIGHT_ALLOW_MISSING:-0}" == "1" ]]; then
    cmd+=(--allow-missing)
  fi
  if (( $# > 0 )); then
    cmd+=("$@")
  fi
  "${cmd[@]}"
}

openrouter_settings_present() {
  local data_root="${CTX_DATA_ROOT:-$HOME/.ctx}"
  local settings_path="${data_root}/settings.json"
  [[ -f "${settings_path}" ]] || return 1
  node -e 'const fs=require("fs"); const path=process.argv[1]; try { const raw=fs.readFileSync(path,"utf8"); const json=JSON.parse(raw); const key=json && json.title_generation && json.title_generation.api_key; if (key && String(key).trim()) process.exit(0); } catch (err) {} process.exit(1);' "${settings_path}"
}

read_openrouter_api_key_from_settings() {
  local data_root="${CTX_DATA_ROOT:-$HOME/.ctx}"
  local settings_path="${data_root}/settings.json"
  [[ -f "${settings_path}" ]] || return 1
  node -e 'const fs=require("fs"); const path=process.argv[1]; try { const raw=fs.readFileSync(path,"utf8"); const json=JSON.parse(raw); const key=json && json.title_generation && json.title_generation.api_key; if (key && String(key).trim()) { process.stdout.write(String(key).trim()); process.exit(0); } } catch (err) {} process.exit(1);' "${settings_path}"
}

has_openrouter_creds() {
  if [[ -n "${OPENROUTER_API_KEY:-}" && -n "${OPENROUTER_BASE_URL:-}" ]]; then
    return 0
  fi
  openrouter_settings_present
}

csv_contains_provider() {
  local csv="${1:-}"
  local provider="${2:-}"
  local value
  IFS=',' read -r -a _providers <<<"${csv}"
  for value in "${_providers[@]:-}"; do
    value="${value// /}"
    if [[ "$value" == "$provider" ]]; then
      return 0
    fi
  done
  return 1
}

csv_remove_provider() {
  local csv="${1:-}"
  local provider="${2:-}"
  local value
  local out=()
  IFS=',' read -r -a _providers <<<"${csv}"
  for value in "${_providers[@]:-}"; do
    value="${value// /}"
    if [[ -z "$value" || "$value" == "$provider" ]]; then
      continue
    fi
    out+=("$value")
  done
  local result=""
  if (( ${#out[@]} > 0 )); then
    local old_ifs="${IFS}"
    IFS=','
    result="${out[*]}"
    IFS="${old_ifs}"
  fi
  printf '%s' "$result"
}

matrix_has_archive_target() {
  local matrix_path="$1"
  local provider_id="$2"
  local target_key="$3"
  node -e '
const fs = require("node:fs");
const [matrixPath, providerId, targetKey] = process.argv.slice(1);
const matrix = JSON.parse(fs.readFileSync(matrixPath, "utf8"));
const provider = (matrix.providers || []).find((row) => row.id === providerId);
if (!provider) process.exit(1);
const managed = provider.managed_install || {};
const targets = managed.targets || {};
if (!targets[targetKey]) process.exit(1);
process.exit(0);
' "$matrix_path" "$provider_id" "$target_key"
}

default_e2e_bundle_root() {
  if [[ -n "${CTX_E2E_BUNDLE_ROOT:-}" ]]; then
    printf '%s' "${CTX_E2E_BUNDLE_ROOT}"
    return
  fi
  printf '%s' "${volatile_artifacts_dir}/ctx-e2e"
}

ensure_endpoint_ui_bundles() {
  if [[ ! -x "${bundle_script}" ]]; then
    echo "missing bundle script: ${bundle_script}" >&2
    exit 1
  fi

  local cache_key="${CTX_E2E_CACHE_KEY:-endpoint-ui}"
  local cache_root
  cache_root="$(default_e2e_bundle_root)"
  mkdir -p "${cache_root}"
  local canonical_bundle_dir="${repo_root}/apps/desktop/src-tauri/bundles"
  # Keep fresh E2E bundle dirs under a home/cache root so containerized validation
  # can see them reliably on macOS hosts.
  local bundle_dir="${CTX_E2E_BUNDLE_DIR:-${cache_root}/bundles-${cache_key}}"
  local first_pass_providers="${CTX_E2E_ENDPOINT_BUNDLE_PROVIDERS:-acp-crp-bridge,codex,cline,copilot,gemini,goose,openhands,qwen,pi,opencode,mistral,droid,kimi}"
  local matrix_json="${CTX_BUNDLE_MATRIX_JSON:-${repo_root}/crates/ctx-http/src/provider_matrix.json}"
  local canonical_runtime_lock="${repo_root}/apps/desktop/src-tauri/bundles/runtime_lock.v2.json"
  local bundle_build_dir="${CTX_E2E_BUNDLE_BUILD_DIR:-${volatile_artifacts_dir}/ctx-e2e-build/${cache_key}}"
  local cargo_target_dir="${CTX_E2E_CARGO_TARGET_DIR:-${volatile_targets_dir}/ctx-e2e/${cache_key}}"
  local cargo_home_dir="${CTX_E2E_CARGO_HOME:-${volatile_artifacts_dir}/ctx-e2e-cargo-home/${cache_key}}"

  local bundle_skip_images="${CTX_E2E_ENDPOINT_SKIP_BUNDLE_IMAGES:-0}"
  local bundle_harness_image="${CTX_E2E_ENDPOINT_BUNDLE_HARNESS_IMAGE:-1}"
  local bundle_include_bridge="${CTX_E2E_ENDPOINT_BUNDLE_INCLUDE_BRIDGE:-1}"
  local bundle_local_adapters="${CTX_E2E_ENDPOINT_BUNDLE_LOCAL_ADAPTERS:-true}"
  local bundle_build_local_adapters="${CTX_E2E_ENDPOINT_BUNDLE_BUILD_LOCAL_ADAPTERS:-1}"
  local append_local_adapters="${CTX_E2E_ENDPOINT_BUNDLE_APPEND_LOCAL_ADAPTERS:-auto}"
  local append_build_local_adapters="${CTX_E2E_ENDPOINT_BUNDLE_APPEND_BUILD_LOCAL_ADAPTERS:-0}"
  local bundle_append_linux_targets="${CTX_E2E_ENDPOINT_APPEND_LINUX_TARGETS:-1}"
  if [[ "${bundle_dir}" == "${canonical_bundle_dir}" && "${CTX_E2E_ALLOW_CANONICAL_BUNDLES:-0}" != "1" ]]; then
    echo "refusing to use canonical desktop bundles dir for e2e: ${bundle_dir}" >&2
    echo "set CTX_E2E_BUNDLE_DIR to an isolated path (or CTX_E2E_ALLOW_CANONICAL_BUNDLES=1 to override intentionally)" >&2
    exit 1
  fi

  mkdir -p "${bundle_build_dir}" "${cargo_target_dir}" "${cargo_home_dir}"
  mkdir -p "${bundle_dir}"
  rm -rf "${bundle_dir}/providers" "${bundle_dir}/runtimes" "${bundle_dir}/images"
  rm -f "${bundle_dir}/manifest.json"

  echo "preparing bundled provider runtimes for endpoint-ui suite (providers: ${first_pass_providers})"
  CTX_BUNDLE_DIR="${bundle_dir}" \
  CTX_BUNDLE_BUILD_DIR="${bundle_build_dir}" \
  CTX_BUNDLE_MATRIX_JSON="${matrix_json}" \
  CTX_BUNDLE_ONLY_PROVIDERS="${first_pass_providers}" \
  CTX_BUNDLE_SKIP_IMAGES="${bundle_skip_images}" \
  CTX_BUNDLE_HARNESS_IMAGE="${bundle_harness_image}" \
  CTX_BUNDLE_INCLUDE_BRIDGE="${bundle_include_bridge}" \
  CTX_BUNDLE_LOCAL_ADAPTERS="${bundle_local_adapters}" \
  CTX_BUNDLE_BUILD_LOCAL_ADAPTERS="${bundle_build_local_adapters}" \
  CTX_BUNDLE_USE_ACP_SHIMS="1" \
  CARGO_TARGET_DIR="${cargo_target_dir}" \
  CARGO_HOME="${cargo_home_dir}" \
  "${bundle_script}" >/dev/null

  if [[ "${OSTYPE:-}" == darwin* && "${bundle_append_linux_targets}" != "0" ]]; then
    local linux_arch="x86_64"
    local append_providers="${first_pass_providers}"
    local codex_append_fallback_arch=""
    if [[ "$(uname -m)" == "arm64" ]]; then
      linux_arch="aarch64"
      if csv_contains_provider "${first_pass_providers}" "codex"; then
        if ! matrix_has_archive_target "${matrix_json}" "codex" "linux-aarch64"; then
          if matrix_has_archive_target "${matrix_json}" "codex" "linux-x86_64"; then
            append_providers="$(csv_remove_provider "${append_providers}" "codex")"
            codex_append_fallback_arch="x86_64"
            echo "warn: codex missing linux/aarch64 target in matrix; appending codex from linux/x86_64 fallback" >&2
          else
            echo "error: codex missing linux/aarch64 and linux/x86_64 targets in matrix: ${matrix_json}" >&2
            exit 1
          fi
        fi
      fi
    fi
    if [[ -n "${append_providers}" ]]; then
      CTX_BUNDLE_DIR="${bundle_dir}" \
      CTX_BUNDLE_BUILD_DIR="${bundle_build_dir}" \
      CTX_BUNDLE_MATRIX_JSON="${matrix_json}" \
      CTX_BUNDLE_APPEND="1" \
      CTX_BUNDLE_OS="linux" \
      CTX_BUNDLE_ARCH="${linux_arch}" \
      CTX_BUNDLE_ONLY_PROVIDERS="${append_providers}" \
      CTX_BUNDLE_SKIP_IMAGES="1" \
      CTX_BUNDLE_HARNESS_IMAGE="0" \
      CTX_BUNDLE_INCLUDE_BRIDGE="${bundle_include_bridge}" \
      CTX_BUNDLE_LOCAL_ADAPTERS="${append_local_adapters}" \
      CTX_BUNDLE_BUILD_LOCAL_ADAPTERS="${append_build_local_adapters}" \
      CTX_BUNDLE_USE_ACP_SHIMS="1" \
      CARGO_TARGET_DIR="${cargo_target_dir}" \
      CARGO_HOME="${cargo_home_dir}" \
      "${bundle_script}" >/dev/null
    fi

    if [[ -n "${codex_append_fallback_arch}" ]]; then
      CTX_BUNDLE_DIR="${bundle_dir}" \
      CTX_BUNDLE_BUILD_DIR="${bundle_build_dir}" \
      CTX_BUNDLE_MATRIX_JSON="${matrix_json}" \
      CTX_BUNDLE_APPEND="1" \
      CTX_BUNDLE_OS="linux" \
      CTX_BUNDLE_ARCH="${codex_append_fallback_arch}" \
      CTX_BUNDLE_ONLY_PROVIDERS="codex" \
      CTX_BUNDLE_SKIP_IMAGES="1" \
      CTX_BUNDLE_HARNESS_IMAGE="0" \
      CTX_BUNDLE_INCLUDE_BRIDGE="0" \
      CTX_BUNDLE_LOCAL_ADAPTERS="off" \
      CTX_BUNDLE_BUILD_LOCAL_ADAPTERS="0" \
      CTX_BUNDLE_USE_ACP_SHIMS="1" \
      CARGO_TARGET_DIR="${cargo_target_dir}" \
      CARGO_HOME="${cargo_home_dir}" \
      "${bundle_script}" >/dev/null
    fi
  fi

  local bundled_runtime_lock="${bundle_dir}/runtime_lock.v2.json"
  local runtime_lock_arch="x86_64"
  if [[ "$(uname -m)" == "arm64" ]]; then
    runtime_lock_arch="aarch64"
  fi
  if [[ ! -f "${bundled_runtime_lock}" && -f "${canonical_runtime_lock}" ]]; then
    cp "${canonical_runtime_lock}" "${bundled_runtime_lock}"
    echo "copied canonical runtime lock to bundle dir: ${bundled_runtime_lock}"
  fi
  if [[ -f "${bundled_runtime_lock}" && -f "${canonical_runtime_lock}" ]]; then
    node -e '
const fs = require("node:fs");
const [bundlePath, canonicalPath, arch] = process.argv.slice(1);
const bundle = JSON.parse(fs.readFileSync(bundlePath, "utf8"));
const canonical = JSON.parse(fs.readFileSync(canonicalPath, "utf8"));
const bundleComponents = Array.isArray(bundle.components) ? bundle.components : [];
const canonicalComponents = Array.isArray(canonical.components) ? canonical.components : [];
const keyFor = (component) => JSON.stringify([
  component.kind || "",
  component.id || "",
  component.os || "",
  component.arch || "",
  component.variant || "default",
]);
const wanted = canonicalComponents.filter((component) => (
  component.kind === "image"
  && component.id === "ctx-harness"
  && component.os === "linux"
  && component.arch === arch
  && (component.variant || "default") === "default"
));
if (wanted.length === 0) {
  process.exit(0);
}
const existing = new Set(bundleComponents.map(keyFor));
let changed = false;
for (const component of wanted) {
  const key = keyFor(component);
  if (existing.has(key)) continue;
  bundleComponents.push(component);
  existing.add(key);
  changed = true;
}
if (changed) {
  bundle.components = bundleComponents;
  fs.writeFileSync(bundlePath, `${JSON.stringify(bundle, null, 2)}\n`);
  process.stdout.write(`added ctx-harness managed image source to ${bundlePath}\n`);
}
' "${bundled_runtime_lock}" "${canonical_runtime_lock}" "${runtime_lock_arch}"
  fi

  export CTX_BUNDLE_DIR="${bundle_dir}"
  export CTX_E2E_BUNDLED_ONLY="1"
  local bundled_only_provider_csv="${CTX_E2E_BUNDLED_ONLY_PROVIDERS:-${first_pass_providers}}"
  if [[ "${CTX_BUNDLE_BUILD_CODEX_CRP:-0}" == "0" ]]; then
    bundled_only_provider_csv="$(csv_remove_provider "${bundled_only_provider_csv}" "codex")"
  fi
  export CTX_E2E_BUNDLED_ONLY_PROVIDERS="${bundled_only_provider_csv}"
  export CTX_E2E_CARGO_TARGET_DIR="${cargo_target_dir}"
  export CARGO_TARGET_DIR="${cargo_target_dir}"
  export CTX_E2E_CARGO_HOME="${cargo_home_dir}"
  export CTX_BUNDLE_BUILD_DIR="${bundle_build_dir}"
  export CTX_BUNDLE_MATRIX_JSON="${matrix_json}"
  echo "using CTX_BUNDLE_DIR=${CTX_BUNDLE_DIR}"
  echo "using CTX_BUNDLE_BUILD_DIR=${CTX_BUNDLE_BUILD_DIR}"
  echo "using CTX_BUNDLE_MATRIX_JSON=${CTX_BUNDLE_MATRIX_JSON}"
  echo "using CTX_E2E_CARGO_TARGET_DIR=${CTX_E2E_CARGO_TARGET_DIR}"
  echo "using CTX_E2E_CARGO_HOME=${CTX_E2E_CARGO_HOME}"
  echo "enforcing bundled-only runtime resolution for: ${CTX_E2E_BUNDLED_ONLY_PROVIDERS}"
}

ensure_openrouter_creds_for_lane() {
  local lane="${1:-}"
  export OPENROUTER_BASE_URL="${OPENROUTER_BASE_URL:-https://openrouter.ai/api/v1}"

  if [[ -z "${OPENROUTER_API_KEY:-}" ]]; then
    OPENROUTER_API_KEY="$(read_openrouter_api_key_from_settings || true)"
    export OPENROUTER_API_KEY
  fi

  if [[ -z "${OPENROUTER_API_KEY:-}" ]]; then
    if [[ -n "${CI:-}" ]]; then
      echo "missing OpenRouter API key for ${lane}; set OPENROUTER_API_KEY or configure title_generation in ${CTX_DATA_ROOT:-$HOME/.ctx}/settings.json" >&2
      exit 1
    fi
    echo "skipping ${lane}; missing OpenRouter API key (set OPENROUTER_API_KEY or configure title_generation in ${CTX_DATA_ROOT:-$HOME/.ctx}/settings.json)" >&2
    exit 0
  fi
}

run_web_playwright_suite() {
  local web_root="${repo_root}/apps/web"
  local playwright_bin="${web_root}/node_modules/.bin/playwright"
  echo "ensuring locked playwright install in ${web_root}" >&2
  (
    cd "${web_root}"
    pnpm install --frozen-lockfile >/dev/null
  )
  if [[ ! -x "${playwright_bin}" ]]; then
    echo "missing playwright binary in ${web_root} after locked install; run 'bash -lc \"cd ${web_root} && pnpm install --frozen-lockfile\"'" >&2
    exit 1
  fi
  if ! (
    cd "${web_root}"
    node -e "require.resolve('playwright/package.json')"
  ) >/dev/null 2>&1; then
    echo "playwright package is not resolvable from ${web_root} after locked install; run 'bash -lc \"cd ${web_root} && pnpm install --frozen-lockfile\"'" >&2
    exit 1
  fi

  (
    cd "${web_root}"
    "${playwright_bin}" test -c playwright.config.ts "$@"
  )
}

run_linux_arm_runtime_install_lane() {
  local lane="$1"
  local allow_failures="$2"

  ensure_openrouter_creds_for_lane "${lane}"

  local lane_key
  if [[ "${lane}" == "linux-arm-critical" ]]; then
    lane_key="critical"
  elif [[ "${lane}" == "linux-arm-nightly" ]]; then
    lane_key="nightly"
  else
    echo "unsupported linux-arm lane: ${lane}" >&2
    exit 2
  fi

  local lane_report_root="${CTX_E2E_LINUX_ARM_REPORT_DIR:-/tmp/ctx-linux-arm-provider-reliability}"
  mkdir -p "${lane_report_root}"
  local preflight_report="${CTX_E2E_LINUX_ARM_PREFLIGHT_REPORT:-${lane_report_root}/${lane_key}-preflight-report.json}"
  local smoke_report="${CTX_E2E_INSTALL_SMOKE_REPORT_PATH:-${lane_report_root}/${lane_key}-runtime-install-smoke-report.json}"

  local provider_csv="${CTX_E2E_INSTALL_SMOKE_PROVIDERS:-}"
  local matrix_payload_json
  matrix_payload_json="$(node "${repo_root}/scripts/linux_arm_provider_reliability_matrix.cjs" --lane "${lane_key}" --json)"
  local default_provider_csv
  default_provider_csv="$(node -e 'const payload = JSON.parse(process.argv[1]); const ids = Array.isArray(payload.provider_ids) ? payload.provider_ids : []; process.stdout.write(ids.join(","));' "${matrix_payload_json}")"
  local expected_environment
  expected_environment="$(node -e 'const payload = JSON.parse(process.argv[1]); process.stdout.write(String(payload.expected_environment || "").trim());' "${matrix_payload_json}")"
  local expected_network_mode
  expected_network_mode="$(node -e 'const payload = JSON.parse(process.argv[1]); process.stdout.write(String(payload.expected_network_mode || "").trim());' "${matrix_payload_json}")"
  local model_override_lines
  model_override_lines="$(node -e 'const payload = JSON.parse(process.argv[1]); const overrides = payload.model_overrides && typeof payload.model_overrides === "object" ? payload.model_overrides : {}; for (const [providerId, rawModel] of Object.entries(overrides)) { const model = String(rawModel || "").trim(); if (!model) continue; const envName = `CTX_E2E_${providerId.toUpperCase().replace(/[^A-Z0-9]+/g, "_")}_OPENROUTER_MODEL_OVERRIDE`; console.log(`${envName}=${model}`); }' "${matrix_payload_json}")"
  if [[ -z "${provider_csv}" ]]; then
    provider_csv="${default_provider_csv}"
  fi
  if [[ -z "${provider_csv}" ]]; then
    echo "linux-arm provider matrix resolved empty provider list for lane ${lane_key}" >&2
    exit 1
  fi
  while IFS= read -r override_entry; do
    [[ -z "${override_entry}" ]] && continue
    local override_key="${override_entry%%=*}"
    local override_value="${override_entry#*=}"
    [[ -z "${override_key}" ]] && continue
    if [[ -n "${!override_key:-}" && "${!override_key}" != "${override_value}" ]]; then
      echo "linux-arm provider matrix override conflict for ${override_key}: existing='${!override_key}' matrix='${override_value}'" >&2
      exit 1
    fi
    export "${override_key}=${override_value}"
  done <<< "${model_override_lines}"

  local preflight_args=(
    "${repo_root}/scripts/linux_arm_provider_preflight.cjs"
    "--lane" "${lane_key}"
    "--report" "${preflight_report}"
  )
  if [[ "${CTX_E2E_LINUX_ARM_PREFLIGHT_STRICT_SIZE_BYTES:-0}" == "1" ]]; then
    preflight_args+=("--strict-size-bytes")
  fi
  node "${preflight_args[@]}"

  # Linux-arm runtime install lanes validate bundled runtime command resolution.
  # Scope bundle preparation to the lane provider set unless caller overrides.
  local bundle_provider_csv="${CTX_E2E_ENDPOINT_BUNDLE_PROVIDERS:-${provider_csv}}"
  if [[ ",${bundle_provider_csv}," != *",acp-crp-bridge,"* ]]; then
    bundle_provider_csv="acp-crp-bridge,${bundle_provider_csv}"
  fi
  export CTX_E2E_ENDPOINT_BUNDLE_PROVIDERS="${bundle_provider_csv}"
  export CTX_E2E_ENDPOINT_SKIP_BUNDLE_IMAGES="${CTX_E2E_ENDPOINT_SKIP_BUNDLE_IMAGES:-1}"
  export CTX_E2E_ENDPOINT_BUNDLE_HARNESS_IMAGE="${CTX_E2E_ENDPOINT_BUNDLE_HARNESS_IMAGE:-0}"
  # Linux-arm push validation runs before newly versioned repo-owned adapter artifacts
  # are published, so source local adapters from the workspace while external lane
  # providers continue using their managed archive paths.
  export CTX_E2E_ENDPOINT_BUNDLE_INCLUDE_BRIDGE="${CTX_E2E_ENDPOINT_BUNDLE_INCLUDE_BRIDGE:-1}"
  export CTX_E2E_ENDPOINT_BUNDLE_LOCAL_ADAPTERS="${CTX_E2E_ENDPOINT_BUNDLE_LOCAL_ADAPTERS:-true}"
  export CTX_E2E_ENDPOINT_BUNDLE_BUILD_LOCAL_ADAPTERS="${CTX_E2E_ENDPOINT_BUNDLE_BUILD_LOCAL_ADAPTERS:-1}"
  export CTX_E2E_ENDPOINT_BUNDLE_APPEND_LOCAL_ADAPTERS="${CTX_E2E_ENDPOINT_BUNDLE_APPEND_LOCAL_ADAPTERS:-off}"
  export CTX_E2E_ENDPOINT_BUNDLE_APPEND_BUILD_LOCAL_ADAPTERS="${CTX_E2E_ENDPOINT_BUNDLE_APPEND_BUILD_LOCAL_ADAPTERS:-0}"
  export CTX_BUNDLE_BUILD_CODEX_CRP="${CTX_BUNDLE_BUILD_CODEX_CRP:-0}"
  ensure_endpoint_ui_bundles

  export CTX_E2E_TIER="endpoint-ui"
  export CTX_E2E_INSTALL_SMOKE_PROVIDERS="${provider_csv}"
  export CTX_E2E_INSTALL_SMOKE_ENVIRONMENT="${CTX_E2E_INSTALL_SMOKE_ENVIRONMENT:-${expected_environment:-host}}"
  export CTX_E2E_INSTALL_SMOKE_NETWORK_MODE="${CTX_E2E_INSTALL_SMOKE_NETWORK_MODE:-${expected_network_mode:-llm_only}}"
  export CTX_E2E_INSTALL_SMOKE_REPORT_PATH="${smoke_report}"
  export CTX_E2E_INSTALL_SMOKE_ALLOW_FAILURES="${allow_failures}"
  # Repo-owned local-adapter providers are bundled into this lane before their
  # stable artifacts exist, so explicit install requests must treat bundled-only
  # providers as preseeded instead of forcing a managed download.
  export CTX_E2E_INSTALL_SMOKE_SKIP_BUNDLED_ONLY_INSTALLS="${CTX_E2E_INSTALL_SMOKE_SKIP_BUNDLED_ONLY_INSTALLS:-1}"
  export OPENAI_API_KEY="${OPENAI_API_KEY:-${OPENROUTER_API_KEY}}"
  export OPENAI_BASE_URL="${OPENAI_BASE_URL:-${OPENROUTER_BASE_URL}}"

  run_web_playwright_suite \
    e2e/runtime-provider-install-openrouter-smoke.spec.ts \
    --workers=1
}

tests=()
add_test() {
  local crate="$1"
  local test_name="$2"
  local file_path="$3"
  if [[ -f "${file_path}" ]]; then
    tests+=("${crate}:${test_name}")
  fi
}

case "${suite}" in
  e2e)
    add_test "ctx-providers" "real_acp_e2e" "${providers_tests_dir}/real_acp_e2e.rs"
    add_test "ctx-http" "harness_container_sandbox_e2e" "${http_tests_dir}/harness_container_sandbox_e2e.rs"
    ;;
  runner)
    add_test "ctx-http" "provider_runner_acp_e2e" "${http_tests_dir}/provider_runner_acp_e2e.rs"
    add_test "ctx-http" "harness_container_sandbox_e2e" "${http_tests_dir}/harness_container_sandbox_e2e.rs"
    ;;
  tokens)
    run_preflight "providers-tokens"
    export CTX_E2E_TIER="tokens"
    if ! has_openrouter_creds; then
      if [[ -n "${CI:-}" ]]; then
        echo "missing OpenRouter credentials; set OPENROUTER_API_KEY/OPENROUTER_BASE_URL or configure title_generation in ${CTX_DATA_ROOT:-$HOME/.ctx}/settings.json" >&2
        exit 1
      fi
      echo "skipping token tests; missing OpenRouter credentials (set OPENROUTER_API_KEY/OPENROUTER_BASE_URL or configure title_generation in ${CTX_DATA_ROOT:-$HOME/.ctx}/settings.json)" >&2
      exit 0
    fi
    local_bridge_manifest="${repo_root}/../external-harnesses/acp-crp-bridge/Cargo.toml"
    export CTX_TOKENS_ACP_CRP_BRIDGE_BIN="${CTX_TOKENS_ACP_CRP_BRIDGE_BIN:-${CTX_E2E_CARGO_TARGET_DIR}/debug/acp-crp-bridge}"
    cargo build --manifest-path "${local_bridge_manifest}" --bin acp-crp-bridge >/dev/null
    if [[ ! -x "${CTX_TOKENS_ACP_CRP_BRIDGE_BIN}" ]]; then
      echo "missing local acp-crp-bridge binary after build: ${CTX_TOKENS_ACP_CRP_BRIDGE_BIN}" >&2
      exit 1
    fi
    add_test "ctx-http" "acp_crp_bridge_tokens_e2e" "${http_tests_dir}/acp_crp_bridge_tokens_e2e.rs"
    ;;
  endpoint-ui)
    run_preflight "providers-endpoint-ui"
    export CTX_E2E_TIER="endpoint-ui"
    export OPENROUTER_BASE_URL="${OPENROUTER_BASE_URL:-https://openrouter.ai/api/v1}"

    if [[ -z "${OPENROUTER_API_KEY:-}" ]]; then
      OPENROUTER_API_KEY="$(read_openrouter_api_key_from_settings || true)"
      export OPENROUTER_API_KEY
    fi

    if [[ -z "${OPENROUTER_API_KEY:-}" ]]; then
      if [[ -n "${CI:-}" ]]; then
        echo "missing OpenRouter API key; set OPENROUTER_API_KEY or configure title_generation in ${CTX_DATA_ROOT:-$HOME/.ctx}/settings.json" >&2
        exit 1
      fi
      echo "skipping endpoint-ui tests; missing OpenRouter API key (set OPENROUTER_API_KEY or configure title_generation in ${CTX_DATA_ROOT:-$HOME/.ctx}/settings.json)" >&2
      exit 0
    fi

    export OPENAI_API_KEY="${OPENAI_API_KEY:-${OPENROUTER_API_KEY}}"
    export OPENAI_BASE_URL="${OPENAI_BASE_URL:-${OPENROUTER_BASE_URL}}"
    export GOOSE_PROVIDER="${GOOSE_PROVIDER:-openrouter}"
    export GOOSE_DISABLE_KEYRING="${GOOSE_DISABLE_KEYRING:-1}"
    export GOOSE_MODEL="${GOOSE_MODEL:-${CTX_E2E_GOOSE_OPENROUTER_MODEL_OVERRIDE:-${CTX_E2E_OPENROUTER_MODEL_OVERRIDE:-openai/gpt-5.2-codex}}}"
    if [[ -n "${CTX_E2E_ENDPOINT_PROVIDERS:-}" && -z "${CTX_E2E_ENDPOINT_BUNDLE_PROVIDERS:-}" ]]; then
      endpoint_bundle_providers="${CTX_E2E_ENDPOINT_PROVIDERS}"
      if [[ ",${endpoint_bundle_providers}," != *",acp-crp-bridge,"* ]]; then
        endpoint_bundle_providers="acp-crp-bridge,${endpoint_bundle_providers}"
      fi
      export CTX_E2E_ENDPOINT_BUNDLE_PROVIDERS="${endpoint_bundle_providers}"
    fi
    export CTX_E2E_ENDPOINT_BUNDLE_HARNESS_IMAGE="${CTX_E2E_ENDPOINT_BUNDLE_HARNESS_IMAGE:-0}"
    export CTX_E2E_ENDPOINT_APPEND_LINUX_TARGETS="${CTX_E2E_ENDPOINT_APPEND_LINUX_TARGETS:-0}"

    ensure_endpoint_ui_bundles

    endpoint_specs=(
      e2e/workbench-endpoint-harness-openrouter-matrix.spec.ts
    )
    if [[ -n "${CTX_E2E_ENDPOINT_SPECS:-}" ]]; then
      IFS=',' read -r -a endpoint_specs <<<"${CTX_E2E_ENDPOINT_SPECS}"
    fi

    run_web_playwright_suite \
      "${endpoint_specs[@]}" \
      --workers=1
    exit 0
    ;;
  provider-api-auth)
    run_preflight "providers-provider-api-auth"
    export CTX_E2E_TIER="provider-api-auth"

    missing_provider_auth_keys=()
    if [[ -z "${CTX_E2E_COPILOT_TOKEN:-}" ]]; then
      missing_provider_auth_keys+=("CTX_E2E_COPILOT_TOKEN")
    fi
    if [[ -z "${CTX_E2E_CURSOR_API_KEY:-}" ]]; then
      missing_provider_auth_keys+=("CTX_E2E_CURSOR_API_KEY")
    fi
    if [[ -z "${CTX_E2E_GEMINI_API_KEY:-}" ]]; then
      missing_provider_auth_keys+=("CTX_E2E_GEMINI_API_KEY")
    fi
    if [[ -z "${GCP_SERVICE_ACCOUNT_JSON:-}" ]]; then
      missing_provider_auth_keys+=("GCP_SERVICE_ACCOUNT_JSON")
    fi
    if [[ -z "${GCP_PROJECT_ID:-}" ]]; then
      missing_provider_auth_keys+=("GCP_PROJECT_ID")
    fi
    if [[ -z "${OPENAI_API_KEY:-}" ]]; then
      missing_provider_auth_keys+=("OPENAI_API_KEY")
    fi
    if [[ -z "${MISTRAL_API_KEY:-}" ]]; then
      missing_provider_auth_keys+=("MISTRAL_API_KEY")
    fi

    if (( ${#missing_provider_auth_keys[@]} > 0 )); then
      missing_keys_csv="$(IFS=,; echo "${missing_provider_auth_keys[*]}")"
      if [[ -n "${CI:-}" ]]; then
        echo "missing ${missing_keys_csv}; set provider API key secrets before running provider-api-auth suite" >&2
        exit 1
      fi
      echo "skipping provider-api-auth tests; missing ${missing_keys_csv}" >&2
      exit 0
    fi

    export CTX_E2E_ENDPOINT_BUNDLE_PROVIDERS="${CTX_E2E_ENDPOINT_BUNDLE_PROVIDERS:-${CTX_E2E_PROVIDER_AUTH_BUNDLE_PROVIDERS:-acp-crp-bridge,codex,copilot,cursor,gemini,mistral}}"
    # Provider API-key auth coverage does not require harness image bundling.
    # Keep this lane independent of Docker buildx health by default.
    export CTX_E2E_ENDPOINT_BUNDLE_HARNESS_IMAGE="${CTX_E2E_ENDPOINT_BUNDLE_HARNESS_IMAGE:-0}"
    # Provider API-key auth checks execute in host-mode web e2e and don't need
    # linux target append/payloads.
    export CTX_E2E_ENDPOINT_APPEND_LINUX_TARGETS="${CTX_E2E_ENDPOINT_APPEND_LINUX_TARGETS:-0}"
    ensure_endpoint_ui_bundles

    provider_api_auth_specs=(
      e2e/workbench-copilot-subscription-token-real.spec.ts
      e2e/workbench-codex-provider-endpoint-openai-real.spec.ts
      e2e/workbench-cursor-provider-api-key-real.spec.ts
      e2e/workbench-gemini-provider-api-key-real.spec.ts
      e2e/workbench-gemini-vertex-provider-api-key-real.spec.ts
      e2e/workbench-mistral-provider-api-key-real.spec.ts
    )
    if [[ -n "${CTX_E2E_PROVIDER_AUTH_SPECS:-}" ]]; then
      IFS=',' read -r -a provider_api_auth_specs <<<"${CTX_E2E_PROVIDER_AUTH_SPECS}"
    fi

    run_web_playwright_suite \
      "${provider_api_auth_specs[@]}" \
      --workers=1
    exit 0
    ;;
  provider-browser-auth)
    run_preflight "providers-provider-browser-auth"
    export CTX_E2E_TIER="provider-browser-auth"
    export CTX_E2E_PROVIDER_BROWSER_AUTH_PERSISTENT="${CTX_E2E_PROVIDER_BROWSER_AUTH_PERSISTENT:-1}"
    export CTX_E2E_PROVIDER_BROWSER_AUTH_STEALTH="${CTX_E2E_PROVIDER_BROWSER_AUTH_STEALTH:-1}"
    export CTX_E2E_PROVIDER_BROWSER_AUTH_HEADLESS="${CTX_E2E_PROVIDER_BROWSER_AUTH_HEADLESS:-0}"
    if [[ "${OSTYPE:-}" == darwin* ]]; then
      export CTX_E2E_PROVIDER_BROWSER_AUTH_CHANNEL="${CTX_E2E_PROVIDER_BROWSER_AUTH_CHANNEL:-chrome}"
      export CTX_E2E_PROVIDER_BROWSER_AUTH_USE_LOCAL_GOOGLE_PROFILE="${CTX_E2E_PROVIDER_BROWSER_AUTH_USE_LOCAL_GOOGLE_PROFILE:-1}"
    fi

    missing_provider_browser_auth_keys=()
    if [[ -z "${GOOGLE_TEST_EMAIL:-}" ]]; then
      missing_provider_browser_auth_keys+=("GOOGLE_TEST_EMAIL")
    fi
    if [[ -z "${GOOGLE_TEST_PASSWORD:-}" ]]; then
      missing_provider_browser_auth_keys+=("GOOGLE_TEST_PASSWORD")
    fi

    if (( ${#missing_provider_browser_auth_keys[@]} > 0 )); then
      missing_keys_csv="$(IFS=,; echo "${missing_provider_browser_auth_keys[*]}")"
      if [[ -n "${CI:-}" ]]; then
        echo "missing ${missing_keys_csv}; set shared Google browser OAuth secrets before running provider-browser-auth suite" >&2
        exit 1
      fi
      echo "skipping provider-browser-auth tests; missing ${missing_keys_csv}" >&2
      exit 0
    fi

    export CTX_E2E_ENDPOINT_BUNDLE_PROVIDERS="${CTX_E2E_ENDPOINT_BUNDLE_PROVIDERS:-${CTX_E2E_PROVIDER_BROWSER_AUTH_BUNDLE_PROVIDERS:-acp-crp-bridge,claude-cli,claude-crp}}"
    export CTX_E2E_ENDPOINT_BUNDLE_HARNESS_IMAGE="${CTX_E2E_ENDPOINT_BUNDLE_HARNESS_IMAGE:-0}"
    export CTX_E2E_ENDPOINT_APPEND_LINUX_TARGETS="${CTX_E2E_ENDPOINT_APPEND_LINUX_TARGETS:-0}"
    ensure_endpoint_ui_bundles

    provider_browser_auth_specs=(
      e2e/workbench-claude-subscription-setup-token-real.spec.ts
    )
    if [[ -n "${CTX_E2E_PROVIDER_BROWSER_AUTH_SPECS:-}" ]]; then
      IFS=',' read -r -a provider_browser_auth_specs <<<"${CTX_E2E_PROVIDER_BROWSER_AUTH_SPECS}"
    fi

    run_web_playwright_suite \
      "${provider_browser_auth_specs[@]}" \
      --workers=1
    exit 0
    ;;
  write-file)
    "${BASH_SOURCE[0]}" tokens
    "${BASH_SOURCE[0]}" endpoint-ui
    "${BASH_SOURCE[0]}" provider-api-auth
    "${BASH_SOURCE[0]}" provider-browser-auth
    exit 0
    ;;
  linux-arm-critical)
    export CTX_E2E_TIER="endpoint-ui"
    run_linux_arm_runtime_install_lane "linux-arm-critical" "0"
    exit 0
    ;;
  linux-arm-nightly)
    export CTX_E2E_TIER="endpoint-ui"
    run_linux_arm_runtime_install_lane "linux-arm-nightly" "1"
    exit 0
    ;;
  *)
    echo "usage: $0 {e2e|runner|tokens|endpoint-ui|provider-api-auth|provider-browser-auth|write-file|linux-arm-critical|linux-arm-nightly}" >&2
    exit 2
    ;;
esac

if [[ "${#tests[@]}" -eq 0 ]]; then
  echo "no provider ${suite} tests found under ${repo_root}/crates" >&2
  exit 1
fi

for entry in "${tests[@]}"; do
  crate="${entry%%:*}"
  test_name="${entry##*:}"
  echo "running ${suite} provider test: ${test_name}"
  cargo test -p "${crate}" --test "${test_name}" -- --ignored --nocapture --test-threads=1
done
