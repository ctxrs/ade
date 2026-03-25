#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../../../.. && pwd)"
WRAPPER="${CTX_REMOTE_BOOTSTRAP_FIXTURE_WRAPPER:-${ROOT}/core/apps/desktop/scripts/test_remote_bootstrap_fixture.sh}"

RUNTIME="${CTX_AUTOMATION_REMOTE_FIXTURE_RUNTIME:-auto}"
CASES_CSV="${CTX_AUTOMATION_REMOTE_AUTH_CASES:-key,password_once,wrong_password}"
MAX_INFRA_RETRIES="${CTX_AUTOMATION_REMOTE_INFRA_RETRIES:-1}"
LOG_DIR="${CTX_AUTOMATION_REMOTE_FIXTURE_LOG_DIR:-}"
PASSTHROUGH_ARGS=()

usage() {
  cat <<'USAGE' >&2
usage:
  test_remote_bootstrap_auth_matrix.sh [--runtime auto|docker|nerdctl] [--cases key,password_once,wrong_password] [--max-infra-retries N] [--log-dir PATH] [-- ...wdio args]
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --runtime)
      RUNTIME="${2:-}"
      shift 2
      ;;
    --cases)
      CASES_CSV="${2:-}"
      shift 2
      ;;
    --max-infra-retries)
      MAX_INFRA_RETRIES="${2:-}"
      shift 2
      ;;
    --log-dir)
      LOG_DIR="${2:-}"
      shift 2
      ;;
    --)
      shift
      PASSTHROUGH_ARGS=("$@")
      break
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      PASSTHROUGH_ARGS+=("$1")
      shift
      ;;
  esac
done

if [[ ! "${MAX_INFRA_RETRIES}" =~ ^[0-9]+$ ]]; then
  echo "error: --max-infra-retries must be a non-negative integer" >&2
  exit 2
fi

if [[ -z "${LOG_DIR}" ]]; then
  LOG_DIR="$(mktemp -d /tmp/ctx-remote-bootstrap-matrix.XXXXXX)"
else
  mkdir -p "${LOG_DIR}"
fi

pick_unused_port() {
  node -e '
    const net = require("node:net");
    const server = net.createServer();
    server.on("error", () => process.exit(1));
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = address && typeof address === "object" ? address.port : 0;
      server.close(() => {
        if (!port) process.exit(2);
        process.stdout.write(String(port));
      });
    });
  '
}

status=0
summary_file="${LOG_DIR}/remote-bootstrap-auth-matrix-summary.txt"

write_summary() {
  {
    echo "remote_bootstrap_auth_matrix"
    echo "runtime=${RUNTIME}"
    echo "cases=${CASES_CSV}"
    echo "status=${status}"
    IFS=',' read -r -a CASES_FOR_SUMMARY <<<"${CASES_CSV}"
    for raw_case in "${CASES_FOR_SUMMARY[@]}"; do
      case_name="$(printf '%s' "${raw_case}" | xargs)"
      [[ -z "${case_name}" ]] && continue
      if [[ -f "${LOG_DIR}/${case_name}/summary.env" ]]; then
        echo "--- ${case_name} ---"
        cat "${LOG_DIR}/${case_name}/summary.env"
      fi
    done
  } >"${summary_file}"
}

trap write_summary EXIT

classify_failure() {
  local log_file="$1"
  if [[ ! -f "${log_file}" ]]; then
    echo "unknown"
    return 0
  fi
  local grep_cmd=("grep" "-Eqi")
  if command -v rg >/dev/null 2>&1; then
    grep_cmd=("rg" "-qi")
  fi
  if "${grep_cmd[@]}" "(ECONNREFUSED|EADDRNOTAVAIL|socket hang up|operation was aborted|Connection refused|WebDriverError|docker is installed but not responding|Cannot connect to the Docker daemon|fixture ssh endpoint did not become ready|no supported container runtime found|xvfb|CN_API_KEY is required|tauri-driver port [0-9]+ is already in use|CrabNebula backend port [0-9]+ is already in use)" "${log_file}"; then
    echo "infra_transient"
    return 0
  fi
  if "${grep_cmd[@]}" "(password-once SSH bootstrap failed|Permission denied|failed to reach remote daemon|unable to select Codex harness|expected connect failure)" "${log_file}"; then
    echo "product_or_contract"
    return 0
  fi
  echo "unknown"
}

run_case() {
  local case_name="$1"
  local case_dir="${LOG_DIR}/${case_name}"
  mkdir -p "${case_dir}"
  local run_log="${case_dir}/run.log"
  local summary="${case_dir}/summary.env"

  local auth_mode="key"
  local test_mode="key"
  local fixture_password=""
  local expect_connect_failure="0"
  local fixture_preseed_key="1"

  case "${case_name}" in
    key)
      auth_mode="key"
      test_mode="key"
      ;;
    password_once)
      auth_mode="password"
      test_mode="password_once"
      fixture_password="CtxFixture-${RANDOM}-${RANDOM}!"
      fixture_preseed_key="0"
      ;;
    wrong_password)
      auth_mode="password"
      test_mode="wrong_password"
      fixture_password="CtxFixture-${RANDOM}-${RANDOM}!"
      expect_connect_failure="1"
      fixture_preseed_key="0"
      ;;
    *)
      echo "error: unsupported case '${case_name}'" >&2
      return 2
      ;;
  esac

  local attempt=0
  local max_attempts=$((MAX_INFRA_RETRIES + 1))
  local status=1
  local classification="unknown"

  while (( attempt < max_attempts )); do
    attempt=$((attempt + 1))
    local tauri_driver_port
    tauri_driver_port="$(pick_unused_port)"
    local tauri_test_backend_port
    tauri_test_backend_port="$(pick_unused_port)"
    local cn_backend_port
    cn_backend_port="$(pick_unused_port)"
    {
      echo "[remote-bootstrap-matrix] case=${case_name} attempt=${attempt}/${max_attempts} runtime=${RUNTIME}"
      echo "[remote-bootstrap-matrix] ports: driver=${tauri_driver_port} backend=${tauri_test_backend_port} cn_backend=${cn_backend_port}"
      local cmd=("${WRAPPER}" --runtime "${RUNTIME}" --auth-mode "${auth_mode}" --test-mode "${test_mode}" --log-dir "${case_dir}/fixture")
      if [[ -n "${fixture_password}" ]]; then
        cmd+=(--password "${fixture_password}")
      fi
      if [[ ${#PASSTHROUGH_ARGS[@]} -gt 0 ]]; then
        cmd+=(-- "${PASSTHROUGH_ARGS[@]}")
      fi

      TAURI_DRIVER_PORT="${tauri_driver_port}" \
      TAURI_TEST_BACKEND_PORT="${tauri_test_backend_port}" \
      CTX_AUTOMATION_CN_BACKEND_PORT="${cn_backend_port}" \
      CTX_AUTOMATION_REMOTE_EXPECT_CONNECT_FAILURE="${expect_connect_failure}" \
      CTX_AUTOMATION_REMOTE_FIXTURE_PRESEED_KEY="${fixture_preseed_key}" \
      CTX_AUTOMATION_REMOTE_WRONG_PASSWORD="definitely-wrong-password" \
      "${cmd[@]}"
    } >"${run_log}" 2>&1 && status=0 || status=$?

    if [[ ${status} -eq 0 ]]; then
      classification="pass"
      break
    fi

    classification="$(classify_failure "${run_log}")"
    if [[ "${classification}" == "infra_transient" && ${attempt} -lt ${max_attempts} ]]; then
      echo "[remote-bootstrap-matrix] retrying case=${case_name} due to infra_transient failure" >>"${run_log}"
      continue
    fi
    break
  done

  {
    echo "case=${case_name}"
    echo "attempts=${attempt}"
    echo "status=${status}"
    echo "classification=${classification}"
    echo "runtime=${RUNTIME}"
  } >"${summary}"

  if [[ ${status} -ne 0 ]]; then
    echo "[remote-bootstrap-matrix] case '${case_name}' failed (classification=${classification}); see ${run_log}" >&2
    return ${status}
  fi

  echo "[remote-bootstrap-matrix] case '${case_name}' passed" >&2
  return 0
}

IFS=',' read -r -a CASES <<<"${CASES_CSV}"
if [[ ${#CASES[@]} -eq 0 ]]; then
  echo "error: --cases produced empty list" >&2
  exit 2
fi

for raw_case in "${CASES[@]}"; do
  case_name="$(printf '%s' "${raw_case}" | xargs)"
  [[ -z "${case_name}" ]] && continue
  rc=0
  run_case "${case_name}" || rc=$?
  if [[ ${rc} -ne 0 ]]; then
    status=${rc}
    break
  fi
done

echo "[remote-bootstrap-matrix] summary: ${summary_file}" >&2
exit ${status}
