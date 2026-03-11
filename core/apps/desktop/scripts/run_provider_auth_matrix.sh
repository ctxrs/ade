#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../../../.. && pwd)"
FIXTURE="${CTX_PROVIDER_AUTH_MATRIX_FIXTURE:-${ROOT}/core/apps/desktop/automation/fixtures/provider_auth_matrix.json}"
SMOKE_SCRIPT="${CTX_PROVIDER_AUTH_MATRIX_SMOKE_SCRIPT:-${ROOT}/scripts/desktop_smoke_with_infisical.sh}"
PREFLIGHT_SCRIPT="${CTX_PROVIDER_AUTH_MATRIX_PREFLIGHT_SCRIPT:-${ROOT}/core/scripts/desktop_e2e_preflight.cjs}"
INFISICAL_ENV="${INFISICAL_ENV:-dev}"
INFISICAL_PROJECT_ID="${INFISICAL_PROJECT_ID:-}"
INFISICAL_CONFIG_FILE="${CTX_PROVIDER_AUTH_MATRIX_INFISICAL_CONFIG_FILE:-${ROOT}/core/.infisical.json}"

if [[ ! -f "${FIXTURE}" ]]; then
  echo "error: fixture not found: ${FIXTURE}" >&2
  exit 1
fi

usage() {
  cat <<'USAGE'
Usage:
  run_provider_auth_matrix.sh [--list] [--lane required|nightly|all] [--cell ID[,ID...]] [--provider ID[,ID...]] [--auth-mode ID[,ID...]] [--daemon-location ID[,ID...]] [--execution-environment ID[,ID...]] [--artifacts-dir DIR] [--dry-run] [--include-deferred]

Options:
  --list                 Print selected matrix cell IDs and exit.
  --lane VALUE           Matrix lane to run (`required`, `nightly`, `all`). Default: required.
  --cell ID[,ID...]      Run only the given cell ID(s). Can be repeated.
  --provider ID[,ID...]  Filter by provider_id. Can be repeated.
  --auth-mode ID[,ID...] Filter by auth_mode. Can be repeated.
  --daemon-location ID[,ID...]       Filter by daemon_location. Can be repeated.
  --execution-environment ID[,ID...] Filter by execution_environment. Can be repeated.
  --artifacts-dir DIR    Output root for cell artifacts.
  --dry-run              Print resolved commands without running them.
  --include-deferred     Execute `support=deferred` cells that define a concrete runner.
USAGE
}

LIST_ONLY=0
DRY_RUN=0
INCLUDE_DEFERRED=0
LANE="required"
ARTIFACTS_DIR=""
RETRY_LIMIT_RAW="${CTX_PROVIDER_AUTH_MATRIX_RETRY_LIMIT:-2}"
RETRY_DELAY_SECONDS_RAW="${CTX_PROVIDER_AUTH_MATRIX_RETRY_DELAY_SECONDS:-15}"
declare -a REQUESTED_CELLS=()
declare -a REQUESTED_PROVIDERS=()
declare -a REQUESTED_AUTH_MODES=()
declare -a REQUESTED_DAEMON_LOCATIONS=()
declare -a REQUESTED_EXECUTION_ENVIRONMENTS=()

if [[ ! "${RETRY_LIMIT_RAW}" =~ ^[0-9]+$ ]]; then
  echo "error: CTX_PROVIDER_AUTH_MATRIX_RETRY_LIMIT must be a non-negative integer (got '${RETRY_LIMIT_RAW}')" >&2
  exit 1
fi
if [[ ! "${RETRY_DELAY_SECONDS_RAW}" =~ ^[0-9]+$ ]]; then
  echo "error: CTX_PROVIDER_AUTH_MATRIX_RETRY_DELAY_SECONDS must be a non-negative integer (got '${RETRY_DELAY_SECONDS_RAW}')" >&2
  exit 1
fi
RETRY_LIMIT="${RETRY_LIMIT_RAW}"
RETRY_DELAY_SECONDS="${RETRY_DELAY_SECONDS_RAW}"

if [[ $# -gt 0 && "$1" == "--" ]]; then
  shift
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --)
      shift
      ;;
    --list)
      LIST_ONLY=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --include-deferred)
      INCLUDE_DEFERRED=1
      shift
      ;;
    --lane)
      if [[ $# -lt 2 ]]; then
        echo "error: --lane requires a value" >&2
        exit 1
      fi
      LANE="$(printf "%s" "$2" | tr '[:upper:]' '[:lower:]')"
      shift 2
      ;;
    --artifacts-dir)
      if [[ $# -lt 2 ]]; then
        echo "error: --artifacts-dir requires a value" >&2
        exit 1
      fi
      ARTIFACTS_DIR="$2"
      shift 2
      ;;
    --cell)
      if [[ $# -lt 2 ]]; then
        echo "error: --cell requires a value" >&2
        exit 1
      fi
      IFS=',' read -r -a CELLS <<<"$2"
      for c in "${CELLS[@]}"; do
        trimmed="$(printf "%s" "$c" | tr -d '[:space:]')"
        [[ -n "${trimmed}" ]] && REQUESTED_CELLS+=("${trimmed}")
      done
      shift 2
      ;;
    --provider)
      if [[ $# -lt 2 ]]; then
        echo "error: --provider requires a value" >&2
        exit 1
      fi
      IFS=',' read -r -a PROVIDERS <<<"$2"
      for p in "${PROVIDERS[@]}"; do
        trimmed="$(printf "%s" "$p" | tr -d '[:space:]')"
        [[ -n "${trimmed}" ]] && REQUESTED_PROVIDERS+=("${trimmed}")
      done
      shift 2
      ;;
    --auth-mode)
      if [[ $# -lt 2 ]]; then
        echo "error: --auth-mode requires a value" >&2
        exit 1
      fi
      IFS=',' read -r -a AUTH_MODES <<<"$2"
      for mode in "${AUTH_MODES[@]}"; do
        trimmed="$(printf "%s" "$mode" | tr -d '[:space:]')"
        [[ -n "${trimmed}" ]] && REQUESTED_AUTH_MODES+=("${trimmed}")
      done
      shift 2
      ;;
    --daemon-location)
      if [[ $# -lt 2 ]]; then
        echo "error: --daemon-location requires a value" >&2
        exit 1
      fi
      IFS=',' read -r -a DAEMON_LOCATIONS <<<"$2"
      for location in "${DAEMON_LOCATIONS[@]}"; do
        trimmed="$(printf "%s" "$location" | tr -d '[:space:]')"
        [[ -n "${trimmed}" ]] && REQUESTED_DAEMON_LOCATIONS+=("${trimmed}")
      done
      shift 2
      ;;
    --execution-environment)
      if [[ $# -lt 2 ]]; then
        echo "error: --execution-environment requires a value" >&2
        exit 1
      fi
      IFS=',' read -r -a EXECUTION_ENVIRONMENTS <<<"$2"
      for environment in "${EXECUTION_ENVIRONMENTS[@]}"; do
        trimmed="$(printf "%s" "$environment" | tr -d '[:space:]')"
        [[ -n "${trimmed}" ]] && REQUESTED_EXECUTION_ENVIRONMENTS+=("${trimmed}")
      done
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

case "${LANE}" in
  required|nightly|all) ;;
  *)
    echo "error: --lane must be one of required|nightly|all (got '${LANE}')" >&2
    exit 1
    ;;
esac

run_preflight() {
  if [[ "${LIST_ONLY}" -eq 1 ]]; then
    return 0
  fi
  local suite_id="$1"
  local -a cmd=(node "${PREFLIGHT_SCRIPT}" --suite "${suite_id}")
  if [[ "${#REQUESTED_CELLS[@]}" -gt 0 ]]; then
    local cells_csv
    cells_csv="$(IFS=,; echo "${REQUESTED_CELLS[*]}")"
    cmd+=(--cell "${cells_csv}")
  fi
  if [[ "${INCLUDE_DEFERRED}" -eq 1 ]]; then
    cmd+=(--include-deferred)
  fi
  if [[ "${CTX_DESKTOP_E2E_PREFLIGHT_ALLOW_MISSING:-0}" == "1" ]]; then
    cmd+=(--allow-missing)
  fi
  maybe_run_with_infisical "${cmd[@]}"
}

can_run_with_infisical() {
  if [[ "${CTX_PROVIDER_AUTH_MATRIX_USE_INFISICAL:-1}" == "0" ]]; then
    return 1
  fi
  if ! command -v infisical >/dev/null 2>&1; then
    return 1
  fi
  [[ -f "${INFISICAL_CONFIG_FILE}" ]]
}

maybe_run_with_infisical() {
  # An explicit project opts into Infisical; otherwise use supplied credentials.
  if [[ -n "${INFISICAL_PROJECT_ID}" ]] && can_run_with_infisical; then
    infisical run --env "${INFISICAL_ENV}" --projectId "${INFISICAL_PROJECT_ID}" -- "$@"
    return
  fi
  "$@"
}

case "${LANE}" in
  required)
    run_preflight "provider-auth-matrix-required"
    ;;
  nightly)
    run_preflight "provider-auth-matrix-nightly"
    ;;
  all)
    run_preflight "provider-auth-matrix-required"
    run_preflight "provider-auth-matrix-nightly"
    ;;
esac

if [[ -z "${ARTIFACTS_DIR}" ]]; then
  RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
  ARTIFACTS_DIR="${ROOT}/core/apps/desktop/automation/artifacts/provider-auth-matrix/${RUN_ID}"
fi

declare -a CELL_LINES=()
while IFS= read -r line; do
  CELL_LINES+=("$line")
done < <(
  node -e '
const fs = require("node:fs");
const fixture = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const lane = String(process.argv[2] || "required").trim().toLowerCase();
const cells = Array.isArray(fixture.cells) ? fixture.cells : [];
const encode = (value) => {
  const text = String(value ?? "");
  return text.length > 0 ? text : "__EMPTY__";
};
for (const cell of cells) {
  if (!cell || typeof cell !== "object") continue;
  const cellLane = String(cell.lane || "").trim().toLowerCase();
  if (lane !== "all" && cellLane !== lane) continue;
  const runner = cell.runner && typeof cell.runner === "object" ? cell.runner : {};
  const extraEnv = runner.extra_env && typeof runner.extra_env === "object" ? runner.extra_env : {};
  const prerequisites = Array.isArray(cell.prerequisites) ? cell.prerequisites : [];
  const daemonLocation = String(cell.daemon_location || "");
  const executionEnvironment = String(cell.execution_environment || "");
  const executionTopology = daemonLocation && executionEnvironment
    ? `${daemonLocation}_${executionEnvironment}`
    : "";
  process.stdout.write([
    String(cell.id || ""),
    String(cell.provider_id || ""),
    String(cell.auth_mode || ""),
    daemonLocation,
    executionEnvironment,
    executionTopology,
    String(cell.support || ""),
    String(cell.lane || ""),
    String(runner.kind || "none"),
    encode(runner.spec || ""),
    encode(runner.scenarios || ""),
    JSON.stringify(extraEnv),
    JSON.stringify(prerequisites),
    encode(cell.skip_reason || ""),
  ].join("\t") + "\n");
}
' "${FIXTURE}" "${LANE}"
)

if [[ "${#CELL_LINES[@]}" -eq 0 ]]; then
  echo "error: no matrix cells found for lane '${LANE}'" >&2
  exit 1
fi

is_selected_cell() {
  local id="$1"
  if [[ "${#REQUESTED_CELLS[@]}" -eq 0 ]]; then
    return 0
  fi
  local requested=""
  for requested in "${REQUESTED_CELLS[@]}"; do
    if [[ "${requested}" == "${id}" ]]; then
      return 0
    fi
  done
  return 1
}

matches_requested_value() {
  local value="$1"
  shift
  local -a requested=("$@")
  if [[ "${#requested[@]}" -eq 0 ]]; then
    return 0
  fi
  local candidate=""
  for candidate in "${requested[@]}"; do
    if [[ "${candidate}" == "${value}" ]]; then
      return 0
    fi
  done
  return 1
}

is_selected_row() {
  local id="$1"
  local provider_id="$2"
  local auth_mode="$3"
  local daemon_location="$4"
  local execution_environment="$5"
  if ! is_selected_cell "${id}"; then
    return 1
  fi
  if [[ "${#REQUESTED_PROVIDERS[@]}" -gt 0 ]]; then
    if ! matches_requested_value "${provider_id}" "${REQUESTED_PROVIDERS[@]}"; then
      return 1
    fi
  fi
  if [[ "${#REQUESTED_AUTH_MODES[@]}" -gt 0 ]]; then
    if ! matches_requested_value "${auth_mode}" "${REQUESTED_AUTH_MODES[@]}"; then
      return 1
    fi
  fi
  if [[ "${#REQUESTED_DAEMON_LOCATIONS[@]}" -gt 0 ]]; then
    if ! matches_requested_value "${daemon_location}" "${REQUESTED_DAEMON_LOCATIONS[@]}"; then
      return 1
    fi
  fi
  if [[ "${#REQUESTED_EXECUTION_ENVIRONMENTS[@]}" -gt 0 ]]; then
    if ! matches_requested_value "${execution_environment}" "${REQUESTED_EXECUTION_ENVIRONMENTS[@]}"; then
      return 1
    fi
  fi
  return 0
}

if [[ "${LIST_ONLY}" -eq 1 ]]; then
  for line in "${CELL_LINES[@]}"; do
    IFS=$'\t' read -r id provider_id auth_mode daemon_location execution_environment execution_topology support lane runner_kind _ _ _ _ skip_reason <<<"${line}"
    if ! is_selected_row "${id}" "${provider_id}" "${auth_mode}" "${daemon_location}" "${execution_environment}"; then
      continue
    fi
    printf "%s\tprovider=%s\tauth=%s\tlocation=%s\texecution_environment=%s\ttopology=%s\tsupport=%s\tlane=%s\trunner=%s\tskip=%s\n" \
      "${id}" "${provider_id}" "${auth_mode}" "${daemon_location}" "${execution_environment}" "${execution_topology}" "${support}" "${lane}" "${runner_kind}" "${skip_reason:-"-"}"
  done
  exit 0
fi

mkdir -p "${ARTIFACTS_DIR}"
SUMMARY="${ARTIFACTS_DIR}/summary.tsv"
printf "cell_id\tstatus\texit_code\tartifact_dir\tprovider_id\tauth_mode\tdaemon_location\texecution_environment\texecution_topology\tlane\tsupport\treason\n" >"${SUMMARY}"

run_cell() {
  local id="$1"
  local provider_id="$2"
  local auth_mode="$3"
  local daemon_location="$4"
  local execution_environment="$5"
  local execution_topology="$6"
  local support="$7"
  local lane="$8"
  local runner_kind="$9"
  local spec="${10}"
  local scenarios="${11}"
  local extra_env_json="${12}"
  local prerequisites_json="${13}"
  local skip_reason="${14}"

  local cell_dir="${ARTIFACTS_DIR}/${id}"
  mkdir -p "${cell_dir}"

  node -e '
const fs = require("node:fs");
const fixture = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const id = process.argv[2];
const cell = (Array.isArray(fixture.cells) ? fixture.cells : []).find((row) => row && row.id === id);
if (!cell) process.exit(2);
fs.writeFileSync(process.argv[3], JSON.stringify(cell, null, 2) + "\n");
' "${FIXTURE}" "${id}" "${cell_dir}/cell.json"

  local effective_skip_reason="${skip_reason}"
  local allow_execution=0
  if [[ "${support}" == "supported" ]]; then
    allow_execution=1
  elif [[ "${support}" == "deferred" && "${INCLUDE_DEFERRED}" -eq 1 ]]; then
    allow_execution=1
  fi
  if [[ "${allow_execution}" -ne 1 ]]; then
    printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n" \
      "${id}" "skip" "0" "${cell_dir}" "${provider_id}" "${auth_mode}" "${daemon_location}" "${execution_environment}" "${execution_topology}" "${lane}" "${support}" "${support}:${effective_skip_reason}" >>"${SUMMARY}"
    return 0
  fi

  local missing_env=""
  missing_env="$(maybe_run_with_infisical node -e '
const { buildRequirement, resolveRequirement } = require(process.argv[2]);
const prereq = JSON.parse(process.argv[1] || "[]");
const missing = [];
for (const key of prereq) {
  const name = String(key || "").trim();
  if (!name) continue;
  const requirement = buildRequirement(name);
  const resolution = resolveRequirement(requirement, {
    env: process.env,
    platform: process.platform,
  });
  if (resolution.status === "missing" || resolution.status === "invalid") {
    missing.push(name);
  }
}
process.stdout.write(missing.join(","));
' "${prerequisites_json}" "${ROOT}/core/scripts/desktop_e2e_secret_contract_lib.cjs")"
  if [[ -n "${missing_env}" ]]; then
    printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n" \
      "${id}" "skip" "0" "${cell_dir}" "${provider_id}" "${auth_mode}" "${daemon_location}" "${execution_environment}" "${execution_topology}" "${lane}" "${support}" "missing_env:${missing_env}" >>"${SUMMARY}"
    return 0
  fi

  local report_path="${cell_dir}/contract_report.json"

  local -a env_kv=(
    "CTX_PROVIDER_AUTH_MATRIX_CELL_ID=${id}"
    "CTX_PROVIDER_AUTH_MATRIX_PROVIDER_ID=${provider_id}"
    "CTX_PROVIDER_AUTH_MATRIX_AUTH_MODE=${auth_mode}"
    "CTX_PROVIDER_AUTH_MATRIX_DAEMON_LOCATION=${daemon_location}"
    "CTX_PROVIDER_AUTH_MATRIX_EXECUTION_ENVIRONMENT=${execution_environment}"
    "CTX_PROVIDER_AUTH_MATRIX_REPORT=${report_path}"
  )

  if [[ "${auth_mode}" == "auth_import" ]]; then
    local import_root="${cell_dir}/auth-import-host"
    mkdir -p "${import_root}"
    env_kv+=(
      "CTX_PROVIDER_AUTH_IMPORT_HOME=${import_root}/home"
      "CTX_PROVIDER_AUTH_IMPORT_XDG_CONFIG_HOME=${import_root}/xdg-config"
      "CTX_PROVIDER_AUTH_IMPORT_XDG_DATA_HOME=${import_root}/xdg-data"
      "CTX_PROVIDER_AUTH_IMPORT_CODEX_HOME=${import_root}/codex-home"
    )
  fi

  if [[ "${runner_kind}" == "desktop_wdio" ]]; then
    local tauri_driver_port=""
    tauri_driver_port="$(node -e '
const net = require("node:net");
const pick = () => new Promise((resolve, reject) => {
  const srv = net.createServer();
  srv.once("error", reject);
  srv.listen(0, "127.0.0.1", () => {
    const addr = srv.address();
    const port = addr && typeof addr === "object" ? addr.port : 0;
    srv.close((err) => (err ? reject(err) : resolve(port)));
  });
});

(async () => {
  process.stdout.write(String(await pick()));
})().catch((err) => {
  console.error(err?.stack || String(err));
  process.exit(1);
});
')"
    if [[ -z "${tauri_driver_port}" ]]; then
      echo "failed to allocate tauri driver port for ${id}" >&2
      exit 2
    fi
    env_kv+=(
      "TAURI_DRIVER_PORT=${tauri_driver_port}"
    )
  fi

  local -a extra_env_lines=()
  while IFS= read -r line; do
    extra_env_lines+=("${line}")
  done < <(node -e '
const extra = JSON.parse(process.argv[1] || "{}");
for (const [k, v] of Object.entries(extra)) {
  process.stdout.write(`${k}=${String(v)}\n`);
}
' "${extra_env_json}")
  if [[ "${#extra_env_lines[@]}" -gt 0 ]]; then
    env_kv+=("${extra_env_lines[@]}")
  fi

  local run_log="${cell_dir}/run.log"
  local max_attempts=$((RETRY_LIMIT + 1))
  local attempt=1
  local exit_code=0
  local status="pass"
  local reason="ok"

  is_retryable_failure() {
    local log_path="$1"
    if [[ ! -f "${log_path}" ]]; then
      return 1
    fi
    # OAuth provider-side credential/verification failures are deterministic and
    # should not consume retry budget.
    if rg -q \
      -e 'OpenAI auth page reported an error: Check your inbox' \
      -e 'Code Incorrect code' \
      -e 'codex oauth login did not succeed' \
      "${log_path}"; then
      return 1
    fi
    rg -q \
      -e 'Request failed with error code ECONNREFUSED' \
      -e 'Request failed with error code UND_ERR_CLOSED' \
      -e 'Request failed with error code UND_ERR_SOCKET' \
      -e 'spawned local daemon is incompatible' \
      -e 'TypeError: fetch failed' \
      -e 'Remote session websocket closed' \
      -e 'failed to send remote message: Trying to work with closed connection' \
      -e 'Failed to start webdriver binary' \
      -e 'test-runner-backend exited \(code=null, signal=SIGKILL\)' \
      -e 'tauri-driver exited \(code=null, signal=SIGKILL\)' \
      -e 'CrabNebula backend port [0-9]+ is already in use' \
      -e 'tauri-driver port [0-9]+ is already in use' \
      "${log_path}"
  }

  if [[ "${DRY_RUN}" -eq 1 ]]; then
    echo
    echo "==> ${id}"
    echo "    provider=${provider_id} auth=${auth_mode} location=${daemon_location} execution_environment=${execution_environment} topology=${execution_topology} lane=${lane}"
    echo "    artifacts=${cell_dir}"
    if [[ "${runner_kind}" == "desktop_wdio" ]]; then
      printf "dry-run env: %s\n" "${env_kv[*]}"
      printf "dry-run cmd: %s -- --spec %s (scenarios=%s)\n" "${SMOKE_SCRIPT}" "${spec}" "${scenarios}"
    elif [[ "${runner_kind}" == "web_playwright" ]]; then
      printf "dry-run env: %s\n" "${env_kv[*]}"
      printf "dry-run cmd: pnpm -C core exec pnpm -C apps/web exec playwright test -c playwright.config.ts %s --workers=1\n" "${spec}"
    else
      printf "dry-run: unsupported runner kind '%s'\n" "${runner_kind}"
    fi
    printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n" \
      "${id}" "dry-run" "0" "${cell_dir}" "${provider_id}" "${auth_mode}" "${daemon_location}" "${execution_environment}" "${execution_topology}" "${lane}" "${support}" "dry-run" >>"${SUMMARY}"
    return 0
  fi

  while true; do
    if [[ "${attempt}" -gt 1 && -f "${run_log}" ]]; then
      mv "${run_log}" "${cell_dir}/run.attempt$((attempt - 1)).log"
    fi
    rm -f "${report_path}"

    echo
    if [[ "${max_attempts}" -gt 1 ]]; then
      echo "==> ${id} (attempt ${attempt}/${max_attempts})"
    else
      echo "==> ${id}"
    fi
    echo "    provider=${provider_id} auth=${auth_mode} location=${daemon_location} execution_environment=${execution_environment} topology=${execution_topology} lane=${lane}"
    echo "    artifacts=${cell_dir}"

    set +e
    if [[ "${runner_kind}" == "desktop_wdio" ]]; then
      (
        cd "${ROOT}"
        maybe_run_with_infisical \
          env "${env_kv[@]}" \
            CTX_AUTOMATION_SCENARIOS="${scenarios}" \
            "${SMOKE_SCRIPT}" -- --spec "${spec}"
      ) 2>&1 | tee "${run_log}"
      exit_code="${PIPESTATUS[0]}"
    elif [[ "${runner_kind}" == "web_playwright" ]]; then
      (
        cd "${ROOT}/core"
        maybe_run_with_infisical \
          env "${env_kv[@]}" pnpm -C apps/web exec playwright test -c playwright.config.ts "${spec}" --workers=1
      ) 2>&1 | tee "${run_log}"
      exit_code="${PIPESTATUS[0]}"
    else
      echo "unsupported runner kind: ${runner_kind}" | tee "${run_log}"
      exit_code=2
    fi
    set -e

    status="pass"
    reason="ok"
    if [[ "${exit_code}" -ne 0 ]]; then
      status="fail"
      reason="command_exit_${exit_code}"
    elif [[ ! -f "${report_path}" ]]; then
      status="fail"
      reason="missing_contract_report"
      exit_code=3
    fi

    if [[ "${status}" == "pass" && -f "${report_path}" ]]; then
      report_result="$(node -e '
const fs = require("node:fs");
const report = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const normalize = (value) => String(value || "").replace(/\s+/g, " ").trim();
process.stdout.write(`${normalize(report.result)}\t${normalize(report.reason)}\n`);
' "${report_path}")"
      IFS=$'\t' read -r report_status report_reason <<<"${report_result}"
      case "${report_status}" in
        pass)
          ;;
        skip)
          status="skip"
          reason="${report_reason:-report_skip}"
          exit_code=0
          ;;
        *)
          status="fail"
          reason="${report_reason:-report_${report_status:-unknown}}"
          exit_code="${exit_code:-4}"
          [[ "${exit_code}" -eq 0 ]] && exit_code=4
          ;;
      esac
    fi

    if [[ "${status}" == "pass" ]]; then
      if [[ "${attempt}" -gt 1 ]]; then
        reason="ok_after_${attempt}_attempts"
      fi
      break
    fi

    if [[ "${attempt}" -lt "${max_attempts}" ]] && is_retryable_failure "${run_log}"; then
      echo "    transient failure detected; retrying in ${RETRY_DELAY_SECONDS}s..."
      attempt=$((attempt + 1))
      sleep "${RETRY_DELAY_SECONDS}"
      continue
    fi
    break
  done

  printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n" \
    "${id}" "${status}" "${exit_code}" "${cell_dir}" "${provider_id}" "${auth_mode}" "${daemon_location}" "${execution_environment}" "${execution_topology}" "${lane}" "${support}" "${reason}" >>"${SUMMARY}"

  [[ "${status}" == "pass" || "${status}" == "skip" ]]
}

fail_count=0
run_count=0
for line in "${CELL_LINES[@]}"; do
  IFS=$'\t' read -r id provider_id auth_mode daemon_location execution_environment execution_topology support lane runner_kind spec scenarios extra_env_json prerequisites_json skip_reason <<<"${line}"
  [[ "${spec}" == "__EMPTY__" ]] && spec=""
  [[ "${scenarios}" == "__EMPTY__" ]] && scenarios=""
  [[ "${skip_reason}" == "__EMPTY__" ]] && skip_reason=""
  if ! is_selected_row "${id}" "${provider_id}" "${auth_mode}" "${daemon_location}" "${execution_environment}"; then
    continue
  fi
  run_count=$((run_count + 1))
  if ! run_cell "${id}" "${provider_id}" "${auth_mode}" "${daemon_location}" "${execution_environment}" "${execution_topology}" "${support}" "${lane}" "${runner_kind}" "${spec}" "${scenarios}" "${extra_env_json}" "${prerequisites_json}" "${skip_reason}"; then
    fail_count=$((fail_count + 1))
  fi
done

if [[ "${run_count}" -eq 0 ]]; then
  echo "error: no matching matrix cells selected" >&2
  exit 1
fi

echo
echo "Provider auth matrix run complete."
echo "  artifacts: ${ARTIFACTS_DIR}"
echo "  summary:   ${SUMMARY}"
echo "  ran:       ${run_count}"
echo "  failed:    ${fail_count}"

if [[ "${fail_count}" -ne 0 ]]; then
  exit 1
fi
