#!/usr/bin/env bash
set -euo pipefail

suite="${1:-}"
if [[ -z "${suite}" ]]; then
  echo "usage: $0 {e2e|runner|tokens|endpoint-ui}" >&2
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

ensure_endpoint_ui_bundles() {
  if [[ ! -x "${bundle_script}" ]]; then
    echo "missing bundle script: ${bundle_script}" >&2
    exit 1
  fi

  local bundle_dir="${CTX_E2E_BUNDLE_DIR:-${repo_root}/apps/desktop/src-tauri/bundles}"
  local first_pass_providers="${CTX_E2E_ENDPOINT_BUNDLE_PROVIDERS:-acp-crp-bridge,codex,qwen,opencode,mistral,goose,kimi,cline,swe-agent,openhands}"
  local run_id="${CTX_E2E_RUN_ID:-$(date +%s)-$$}"
  local bundle_build_dir="${CTX_E2E_BUNDLE_BUILD_DIR:-/tmp/ctx-e2e-bundle-build-${run_id}}"
  local cargo_target_dir="${CTX_E2E_CARGO_TARGET_DIR:-/tmp/ctx-e2e-cargo-${run_id}}"
  local cargo_home_dir="${CTX_E2E_CARGO_HOME:-/tmp/ctx-e2e-cargo-home-${run_id}}"

  mkdir -p "${bundle_build_dir}" "${cargo_target_dir}" "${cargo_home_dir}"

  echo "preparing bundled provider runtimes for endpoint-ui suite (providers: ${first_pass_providers})"
  CTX_BUNDLE_DIR="${bundle_dir}" \
  CTX_BUNDLE_BUILD_DIR="${bundle_build_dir}" \
  CTX_BUNDLE_ONLY_PROVIDERS="${first_pass_providers}" \
  CTX_BUNDLE_SKIP_IMAGES="${CTX_E2E_ENDPOINT_SKIP_BUNDLE_IMAGES:-1}" \
  CTX_BUNDLE_INCLUDE_BRIDGE="1" \
  CTX_BUNDLE_LOCAL_ADAPTERS="auto" \
  CTX_BUNDLE_BUILD_LOCAL_ADAPTERS="0" \
  CTX_BUNDLE_USE_ACP_SHIMS="0" \
  CARGO_TARGET_DIR="${cargo_target_dir}" \
  CARGO_HOME="${cargo_home_dir}" \
  "${bundle_script}" >/dev/null

  export CTX_BUNDLE_DIR="${bundle_dir}"
  export CTX_E2E_BUNDLED_ONLY="1"
  export CTX_E2E_BUNDLED_ONLY_PROVIDERS="${first_pass_providers}"
  export CTX_E2E_CARGO_TARGET_DIR="${cargo_target_dir}"
  export CARGO_TARGET_DIR="${cargo_target_dir}"
  export CTX_E2E_CARGO_HOME="${cargo_home_dir}"
  export CTX_BUNDLE_BUILD_DIR="${bundle_build_dir}"
  echo "using CTX_BUNDLE_DIR=${CTX_BUNDLE_DIR}"
  echo "using CTX_BUNDLE_BUILD_DIR=${CTX_BUNDLE_BUILD_DIR}"
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

    ensure_endpoint_ui_bundles

    (
      cd "${repo_root}/apps/web"
      pnpm exec playwright test -c playwright.config.ts e2e/workbench-endpoint-harness-openrouter-matrix.spec.ts
    )
    exit 0
    ;;
  *)
    echo "usage: $0 {e2e|runner|tokens|endpoint-ui}" >&2
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
