#!/usr/bin/env bash
set -euo pipefail

suite="${1:-}"
if [[ -z "${suite}" ]]; then
  echo "usage: $0 {e2e|runner|tokens|endpoint-ui|provider-api-auth}" >&2
  exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"
http_tests_dir="${repo_root}/crates/ctx-http/tests"
providers_tests_dir="${repo_root}/crates/ctx-providers/tests"
bundle_script="${repo_root}/../scripts/ensure_bundled_harnesses.sh"

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

ensure_endpoint_ui_bundles() {
  if [[ ! -x "${bundle_script}" ]]; then
    echo "missing bundle script: ${bundle_script}" >&2
    exit 1
  fi

  local cache_key="${CTX_E2E_CACHE_KEY:-endpoint-ui}"
  local canonical_bundle_dir="${repo_root}/apps/desktop/src-tauri/bundles"
  local bundle_dir="${CTX_E2E_BUNDLE_DIR:-/tmp/ctx-e2e-bundles-${cache_key}}"
  local first_pass_providers="${CTX_E2E_ENDPOINT_BUNDLE_PROVIDERS:-acp-crp-bridge,codex,gemini,qwen,opencode,mistral,goose,droid,kimi,openhands}"
  local matrix_json="${CTX_BUNDLE_MATRIX_JSON:-${repo_root}/crates/ctx-http/src/provider_matrix.json}"
  local bundle_build_dir="${CTX_E2E_BUNDLE_BUILD_DIR:-/tmp/ctx-e2e-bundle-build-${cache_key}}"
  local cargo_target_dir="${CTX_E2E_CARGO_TARGET_DIR:-/tmp/ctx-e2e-cargo-${cache_key}}"
  local cargo_home_dir="${CTX_E2E_CARGO_HOME:-/tmp/ctx-e2e-cargo-home-${cache_key}}"

  local bundle_skip_images="${CTX_E2E_ENDPOINT_SKIP_BUNDLE_IMAGES:-0}"
  local bundle_podman="${CTX_E2E_ENDPOINT_BUNDLE_PODMAN:-}"
  if [[ -z "${bundle_podman}" && "${OSTYPE:-}" == darwin* ]]; then
    bundle_podman="1"
  fi
  local podman_version="${PODMAN_VERSION:-5.8.0}"
  local podman_archive_url="${PODMAN_ARCHIVE_URL:-}"
  if [[ -z "${podman_archive_url}" && "${bundle_podman}" == "1" && "${OSTYPE:-}" == darwin* ]]; then
    local podman_arch="amd64"
    if [[ "$(uname -m)" == "arm64" ]]; then
      podman_arch="arm64"
    fi
    podman_archive_url="https://github.com/containers/podman/releases/download/v${podman_version}/podman-remote-release-darwin_${podman_arch}.zip"
  fi

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
  CTX_BUNDLE_HARNESS_IMAGE="${CTX_E2E_ENDPOINT_BUNDLE_HARNESS_IMAGE:-1}" \
  CTX_BUNDLE_INCLUDE_BRIDGE="1" \
  CTX_BUNDLE_LOCAL_ADAPTERS="true" \
  CTX_BUNDLE_BUILD_LOCAL_ADAPTERS="1" \
  CTX_BUNDLE_USE_ACP_SHIMS="1" \
  CTX_BUNDLE_PODMAN="${bundle_podman:-0}" \
  PODMAN_VERSION="${podman_version}" \
  PODMAN_ARCHIVE_URL="${podman_archive_url}" \
  PODMAN_BIN_REL="${PODMAN_BIN_REL:-usr/bin/podman}" \
  CARGO_TARGET_DIR="${cargo_target_dir}" \
  CARGO_HOME="${cargo_home_dir}" \
  "${bundle_script}" >/dev/null

  if [[ "${OSTYPE:-}" == darwin* ]]; then
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
      CTX_BUNDLE_INCLUDE_BRIDGE="1" \
      CTX_BUNDLE_LOCAL_ADAPTERS="auto" \
      CTX_BUNDLE_BUILD_LOCAL_ADAPTERS="0" \
      CTX_BUNDLE_USE_ACP_SHIMS="1" \
      CTX_BUNDLE_PODMAN="0" \
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
      CTX_BUNDLE_PODMAN="0" \
      CARGO_TARGET_DIR="${cargo_target_dir}" \
      CARGO_HOME="${cargo_home_dir}" \
      "${bundle_script}" >/dev/null
    fi
  fi

  export CTX_BUNDLE_DIR="${bundle_dir}"
  export CTX_E2E_BUNDLED_ONLY="1"
  export CTX_E2E_BUNDLED_ONLY_PROVIDERS="${first_pass_providers}"
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
    add_test "ctx-http" "harness_container_podman_e2e" "${http_tests_dir}/harness_container_podman_e2e.rs"
    ;;
  runner)
    add_test "ctx-http" "provider_runner_acp_e2e" "${http_tests_dir}/provider_runner_acp_e2e.rs"
    add_test "ctx-http" "harness_container_podman_e2e" "${http_tests_dir}/harness_container_podman_e2e.rs"
    ;;
  tokens)
    export CTX_E2E_TIER="tokens"
    if ! has_openrouter_creds; then
      if [[ -n "${CI:-}" ]]; then
        echo "missing OpenRouter credentials; set OPENROUTER_API_KEY/OPENROUTER_BASE_URL or configure title_generation in ${CTX_DATA_ROOT:-$HOME/.ctx}/settings.json" >&2
        exit 1
      fi
      echo "skipping token tests; missing OpenRouter credentials (set OPENROUTER_API_KEY/OPENROUTER_BASE_URL or configure title_generation in ${CTX_DATA_ROOT:-$HOME/.ctx}/settings.json)" >&2
      exit 0
    fi
    add_test "ctx-http" "acp_crp_bridge_tokens_e2e" "${http_tests_dir}/acp_crp_bridge_tokens_e2e.rs"
    ;;
  endpoint-ui)
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

    ensure_endpoint_ui_bundles

    (
      cd "${repo_root}/apps/web"
      pnpm exec playwright test -c playwright.config.ts \
        e2e/workbench-endpoint-harness-openrouter-matrix.spec.ts \
        --workers=1
    )
    exit 0
    ;;
  provider-api-auth)
    export CTX_E2E_TIER="provider-api-auth"

    missing_provider_auth_keys=()
    if [[ -z "${CTX_E2E_CURSOR_API_KEY:-}" ]]; then
      missing_provider_auth_keys+=("CTX_E2E_CURSOR_API_KEY")
    fi
    if [[ -z "${CTX_E2E_GEMINI_API_KEY:-}" ]]; then
      missing_provider_auth_keys+=("CTX_E2E_GEMINI_API_KEY")
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

    export CTX_E2E_ENDPOINT_BUNDLE_PROVIDERS="${CTX_E2E_ENDPOINT_BUNDLE_PROVIDERS:-${CTX_E2E_PROVIDER_AUTH_BUNDLE_PROVIDERS:-acp-crp-bridge,cursor,gemini}}"
    ensure_endpoint_ui_bundles

    (
      cd "${repo_root}/apps/web"
      pnpm exec playwright test -c playwright.config.ts \
        e2e/workbench-cursor-provider-api-key-real.spec.ts \
        e2e/workbench-gemini-provider-api-key-real.spec.ts \
        --workers=1
    )
    exit 0
    ;;
  *)
    echo "usage: $0 {e2e|runner|tokens|endpoint-ui|provider-api-auth}" >&2
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
