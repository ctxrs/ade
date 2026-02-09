#!/usr/bin/env bash
set -euo pipefail

suite="${1:-}"
if [[ -z "${suite}" ]]; then
  echo "usage: $0 {e2e|runner|tokens}" >&2
  exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"
http_tests_dir="${repo_root}/crates/ctx-http/tests"
providers_tests_dir="${repo_root}/crates/ctx-providers/tests"

openrouter_settings_present() {
  local data_root="${CTX_DATA_ROOT:-$HOME/.ctx}"
  local settings_path="${data_root}/settings.json"
  [[ -f "${settings_path}" ]] || return 1
  node -e 'const fs=require("fs"); const path=process.argv[1]; try { const raw=fs.readFileSync(path,"utf8"); const json=JSON.parse(raw); const key=json && json.title_generation && json.title_generation.api_key; if (key && String(key).trim()) process.exit(0); } catch (err) {} process.exit(1);' "${settings_path}"
}

has_openrouter_creds() {
  if [[ -n "${OPENROUTER_API_KEY:-}" && -n "${OPENROUTER_BASE_URL:-}" ]]; then
    return 0
  fi
  openrouter_settings_present
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
  *)
    echo "usage: $0 {e2e|runner|tokens}" >&2
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
