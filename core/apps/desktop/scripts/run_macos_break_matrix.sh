#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../../../.. && pwd)"
FIXTURE="${ROOT}/core/apps/desktop/automation/fixtures/macos_break_matrix.json"
SMOKE_SCRIPT="${ROOT}/scripts/desktop_smoke_with_infisical.sh"
PREFLIGHT_SCRIPT="${ROOT}/core/scripts/desktop_e2e_preflight.cjs"

if [[ ! -f "${FIXTURE}" ]]; then
  echo "error: fixture not found: ${FIXTURE}" >&2
  exit 1
fi

if [[ ! -x "${SMOKE_SCRIPT}" ]]; then
  echo "error: required runner is missing or not executable: ${SMOKE_SCRIPT}" >&2
  exit 1
fi

usage() {
  cat <<'USAGE'
Usage:
  run_macos_break_matrix.sh [--list] [--case ID[,ID...]] [--artifacts-dir DIR] [--dry-run]

Options:
  --list                 Print available matrix case IDs and exit.
  --case ID[,ID...]      Run only the given case ID(s). Can be repeated.
  --artifacts-dir DIR    Output root for case artifacts.
  --dry-run              Print resolved commands without running them.
USAGE
}

LIST_ONLY=0
DRY_RUN=0
ARTIFACTS_DIR=""
declare -a REQUESTED_CASES=()

if [[ $# -gt 0 && "$1" == "--" ]]; then
  shift
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)
      LIST_ONLY=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --artifacts-dir)
      if [[ $# -lt 2 ]]; then
        echo "error: --artifacts-dir requires a value" >&2
        exit 1
      fi
      ARTIFACTS_DIR="$2"
      shift 2
      ;;
    --case)
      if [[ $# -lt 2 ]]; then
        echo "error: --case requires a value" >&2
        exit 1
      fi
      IFS=',' read -r -a CASES <<<"$2"
      for c in "${CASES[@]}"; do
        c_trimmed="$(printf "%s" "$c" | tr -d '[:space:]')"
        [[ -n "$c_trimmed" ]] && REQUESTED_CASES+=("$c_trimmed")
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

run_preflight() {
  local -a cmd=(node "${PREFLIGHT_SCRIPT}" --suite "macos-break-matrix")
  if [[ "${#REQUESTED_CASES[@]}" -gt 0 ]]; then
    local cases_csv
    cases_csv="$(IFS=,; echo "${REQUESTED_CASES[*]}")"
    cmd+=(--case "${cases_csv}")
  fi
  if [[ "${CTX_DESKTOP_E2E_PREFLIGHT_ALLOW_MISSING:-0}" == "1" ]]; then
    cmd+=(--allow-missing)
  fi
  "${cmd[@]}"
}

if [[ -z "${ARTIFACTS_DIR}" ]]; then
  RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
  ARTIFACTS_DIR="${ROOT}/core/apps/desktop/automation/artifacts/macos-break-matrix/${RUN_ID}"
fi

declare -a CASE_LINES=()
while IFS= read -r line; do
  CASE_LINES+=("$line")
done < <(
  node -e '
const fs = require("fs");
const fixture = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
for (const c of fixture.cases || []) {
  process.stdout.write([c.id, c.title, c.spec, c.scenarios].join("\t") + "\n");
}
' "${FIXTURE}"
)

if [[ "${#CASE_LINES[@]}" -eq 0 ]]; then
  echo "error: no cases found in fixture ${FIXTURE}" >&2
  exit 1
fi

if [[ "${LIST_ONLY}" -eq 1 ]]; then
  for line in "${CASE_LINES[@]}"; do
    IFS=$'\t' read -r id title _ _ <<<"${line}"
    printf "%s\t%s\n" "${id}" "${title}"
  done
  exit 0
fi

run_preflight()

is_selected_case() {
  local id="$1"
  if [[ "${#REQUESTED_CASES[@]}" -eq 0 ]]; then
    return 0
  fi
  local requested=""
  for requested in "${REQUESTED_CASES[@]}"; do
    if [[ "${requested}" == "${id}" ]]; then
      return 0
    fi
  done
  return 1
}

mkdir -p "${ARTIFACTS_DIR}"
SUMMARY="${ARTIFACTS_DIR}/summary.tsv"
printf "case_id\tstatus\texit_code\tartifact_dir\ttitle\n" >"${SUMMARY}"

run_case() {
  local id="$1"
  local title="$2"
  local spec="$3"
  local scenarios="$4"
  local case_dir="${ARTIFACTS_DIR}/${id}"

  mkdir -p "${case_dir}"
  node -e '
const fs = require("fs");
const fixture = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const id = process.argv[2];
const c = (fixture.cases || []).find((x) => x.id === id);
if (!c) process.exit(2);
fs.writeFileSync(process.argv[3], JSON.stringify(c, null, 2) + "\n");
' "${FIXTURE}" "${id}" "${case_dir}/case.json"

  local -a env_kv=(
    "CTX_AUTOMATION_SCENARIOS=${scenarios}"
    "CTX_AUTOMATION_CN_BACKEND_LOG=${case_dir}/test-runner-backend.log"
    "CTX_AUTOMATION_CN_DRIVER_LOG=${case_dir}/tauri-driver.log"
    "CTX_AUTOMATION_WDIO_LOG_LEVEL=${CTX_AUTOMATION_WDIO_LOG_LEVEL:-info}"
    "CTX_AUTOMATION_MOCHA_TIMEOUT_MS=${CTX_AUTOMATION_MOCHA_TIMEOUT_MS:-900000}"
    "CTX_AUTOMATION_SKIP_APP_BUILD=${CTX_AUTOMATION_SKIP_APP_BUILD:-1}"
    "CTX_CONTAINER_LIFECYCLE_RESUME_REPORT=${case_dir}/container-lifecycle-resume.json"
    "CTX_HARNESS_MATRIX_REPORT=${case_dir}/harness-install-probe-matrix.json"
    "CTX_REMOTE_CONTAINER_CONTRACT_REPORT=${case_dir}/remote-container-contract.json"
    "CTX_UPDATER_NATIVE_SMOKE_REPORT=${case_dir}/updater-native-smoke.json"
  )

  echo
  echo "==> ${id}: ${title}"
  echo "    spec=${spec}"
  echo "    scenarios=${scenarios}"
  echo "    artifacts=${case_dir}"

  if [[ "${DRY_RUN}" -eq 1 ]]; then
    printf "dry-run env: %s\n" "${env_kv[*]}"
    printf "dry-run cmd: %s -- --spec %s\n" "${SMOKE_SCRIPT}" "${spec}"
    printf "%s\t%s\t%s\t%s\t%s\n" "${id}" "dry-run" "0" "${case_dir}" "${title}" >>"${SUMMARY}"
    return 0
  fi

  set +e
  (
    cd "${ROOT}"
    env "${env_kv[@]}" "${SMOKE_SCRIPT}" -- --spec "${spec}"
  ) 2>&1 | tee "${case_dir}/wdio.log"
  local exit_code="${PIPESTATUS[0]}"
  set -e

  local status="pass"
  if [[ "${exit_code}" -ne 0 ]]; then
    status="fail"
  fi
  printf "%s\t%s\t%s\t%s\t%s\n" "${id}" "${status}" "${exit_code}" "${case_dir}" "${title}" >>"${SUMMARY}"
  return "${exit_code}"
}

fail_count=0
ran_count=0
for line in "${CASE_LINES[@]}"; do
  IFS=$'\t' read -r id title spec scenarios <<<"${line}"
  if ! is_selected_case "${id}"; then
    continue
  fi
  ran_count=$((ran_count + 1))
  if ! run_case "${id}" "${title}" "${spec}" "${scenarios}"; then
    fail_count=$((fail_count + 1))
  fi
done

if [[ "${ran_count}" -eq 0 ]]; then
  echo "error: no matching cases selected" >&2
  exit 1
fi

echo
echo "Break matrix run complete."
echo "  artifacts: ${ARTIFACTS_DIR}"
echo "  summary:   ${SUMMARY}"
echo "  ran:       ${ran_count}"
echo "  failed:    ${fail_count}"

if [[ "${fail_count}" -ne 0 ]]; then
  exit 1
fi
