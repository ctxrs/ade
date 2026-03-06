#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../../../.. && pwd)"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
ARTIFACT_DIR="${CTX_REMOTE_CI_ARTIFACT_DIR:-${ROOT}/core/apps/desktop/automation/artifacts/remote-contracts/${RUN_ID}}"
RUN_CONTAINER="${CTX_REMOTE_CI_RUN_CONTAINER:-1}"
STRICT_REQUIRED="${CTX_AUTOMATION_REMOTE_STRICT:-1}"
ALLOW_SKIP="${CTX_AUTOMATION_REMOTE_ALLOW_SKIP:-0}"
REQUIRE_FIRST_TURN_SUCCESS="${CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS:-0}"
DRY_RUN=0

usage() {
  cat <<'USAGE'
usage:
  test_remote_real_ci.sh [--artifacts-dir DIR] [--run-container 1|0] [--dry-run]

Runs the remote host bootstrap contract and, optionally, the remote container
contract against the configured remote fixture contract.

important env:
  CTX_REMOTE_CI_ARTIFACT_DIR
  CTX_REMOTE_CI_RUN_CONTAINER=1|0
  CTX_AUTOMATION_REMOTE_STRICT=1|0
  CTX_AUTOMATION_REMOTE_ALLOW_SKIP=1|0
  CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS=1|0
USAGE
}

if [[ $# -gt 0 && "$1" == "--" ]]; then
  shift
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --artifacts-dir)
      if [[ $# -lt 2 ]]; then
        echo "error: --artifacts-dir requires a value" >&2
        exit 2
      fi
      ARTIFACT_DIR="$2"
      shift 2
      ;;
    --run-container)
      if [[ $# -lt 2 ]]; then
        echo "error: --run-container requires 1 or 0" >&2
        exit 2
      fi
      RUN_CONTAINER="$2"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "${RUN_CONTAINER}" != "0" && "${RUN_CONTAINER}" != "1" ]]; then
  echo "error: --run-container must be 1 or 0" >&2
  exit 2
fi

mkdir -p "${ARTIFACT_DIR}"

HOST_DIR="${ARTIFACT_DIR}/remote-host"
CONTAINER_DIR="${ARTIFACT_DIR}/remote-container"
PRECHECK_JSON="${ARTIFACT_DIR}/preflight.json"
SUMMARY_TSV="${ARTIFACT_DIR}/summary.tsv"
SUMMARY_TXT="${ARTIFACT_DIR}/remote-real-ci-summary.txt"

mkdir -p "${HOST_DIR}"
if [[ "${RUN_CONTAINER}" == "1" ]]; then
  mkdir -p "${CONTAINER_DIR}"
fi

echo "[remote-contracts] artifact dir: ${ARTIFACT_DIR}" >&2

run_preflight() {
  if ! CTX_AUTOMATION_REMOTE_STRICT="${STRICT_REQUIRED}" \
    CTX_AUTOMATION_REMOTE_ALLOW_SKIP="${ALLOW_SKIP}" \
    node - "${ROOT}" "${RUN_CONTAINER}" >"${PRECHECK_JSON}" <<'NODE'
const path = require("node:path");
const root = process.argv[2];
const runContainer = process.argv[3] === "1";
const { resolveRemoteFixtureEnv } = require(path.join(root, "core/apps/desktop/automation/helpers/remote_fixture_contract.cjs"));

const host = resolveRemoteFixtureEnv({ lane: "host" });
const container = runContainer ? resolveRemoteFixtureEnv({ lane: "container" }) : null;
const payload = {
  strict_required: host.strictRequired || Boolean(container && container.strictRequired),
  allow_skip: host.allowSkip || Boolean(container && container.allowSkip),
  lanes: {
    host,
    container,
  },
};
process.stdout.write(`${JSON.stringify(payload, null, 2)}\n`);

const failures = [];
if (!host.ready && host.strictRequired) {
  failures.push(host.preflightMessage);
}
if (container && !container.ready && container.strictRequired) {
  failures.push(container.preflightMessage);
}

if (failures.length > 0) {
  for (const failure of failures) {
    process.stderr.write(`[remote-contracts] ${failure}\n`);
  }
  process.exit(2);
}
NODE
  then
    echo "[remote-contracts] strict preflight failed; see ${PRECHECK_JSON}" >&2
    return 2
  fi
}

sanitize_summary_value() {
  printf '%s' "${1:-}" | tr '\t\r\n' '   '
}

classify_report() {
  local report_path="$1"
  local allow_skip="$2"
  node -e '
const fs = require("node:fs");
const reportPath = process.argv[1];
const allowSkip = process.argv[2] === "1";
const report = JSON.parse(fs.readFileSync(reportPath, "utf8"));
const result = String(report.result || (report.skipped ? "skipped" : "unknown")).trim().toLowerCase();
const reason = String(report.reason || report.skip_reason || "").replace(/[\t\r\n]+/g, " ").trim();
if (result === "passed" || result === "pass") {
  process.stdout.write(`pass\t0\t${reason}\n`);
  process.exit(0);
}
if (result === "skipped") {
  const status = allowSkip ? "skipped" : "infra_unavailable";
  const code = allowSkip ? "0" : "86";
  process.stdout.write(`${status}\t${code}\t${reason || "lane skipped"}\n`);
  process.exit(0);
}
process.stdout.write(`fail\t1\t${reason || result || "report marked failed"}\n`);
' "${report_path}" "${allow_skip}"
}

printf "lane\tstatus\texit_code\tartifact_dir\treason\n" >"${SUMMARY_TSV}"

run_lane() {
  local lane="$1"
  local lane_dir="$2"
  local report_path="$3"
  shift 3
  local -a cmd=("$@")
  local wdio_log="${lane_dir}/wdio.log"
  local status="pass"
  local lane_exit=0
  local reason=""

  if [[ "${DRY_RUN}" == "1" ]]; then
    printf "dry-run lane=%s cmd=%s\n" "${lane}" "${cmd[*]}" >"${wdio_log}"
    status="dry-run"
    lane_exit=0
    reason="dry-run"
  else
    touch "${wdio_log}"
    set +e
    (
      cd "${ROOT}"
      "${cmd[@]}"
    ) >"${wdio_log}" 2>&1 &
    local cmd_pid="$!"
    tail -n +1 -f --pid="${cmd_pid}" "${wdio_log}" || true
    wait "${cmd_pid}"
    local cmd_exit="$?"
    set -e

    if [[ -f "${report_path}" ]]; then
      local parsed_status=""
      local parsed_exit=""
      local parsed_reason=""
      IFS=$'\t' read -r parsed_status parsed_exit parsed_reason < <(classify_report "${report_path}" "${ALLOW_SKIP}")
      if [[ "${cmd_exit}" -ne 0 ]]; then
        status="fail"
        lane_exit="${cmd_exit}"
        reason="${parsed_reason:-command exited ${cmd_exit}}"
      else
        status="${parsed_status}"
        lane_exit="${parsed_exit}"
        reason="${parsed_reason}"
      fi
    else
      if [[ "${cmd_exit}" -ne 0 ]]; then
        status="fail"
        lane_exit="${cmd_exit}"
        reason="command exited ${cmd_exit}"
      else
        status="fail"
        lane_exit=1
        reason="missing report ${report_path}"
      fi
    fi
  fi

  printf "%s\t%s\t%s\t%s\t%s\n" \
    "${lane}" \
    "${status}" \
    "${lane_exit}" \
    "${lane_dir}" \
    "$(sanitize_summary_value "${reason}")" >>"${SUMMARY_TSV}"

  if [[ -n "${reason}" ]]; then
    echo "[remote-contracts] ${lane}: ${status} (${reason})" >&2
  else
    echo "[remote-contracts] ${lane}: ${status}" >&2
  fi

  return "${lane_exit}"
}

run_preflight

host_report="${HOST_DIR}/contract-report.json"
host_first_turn="${HOST_DIR}/first-turn.json"
run_lane \
  "remote-host" \
  "${HOST_DIR}" \
  "${host_report}" \
  env \
  "CTX_AUTOMATION_REMOTE_STRICT=${STRICT_REQUIRED}" \
  "CTX_AUTOMATION_REMOTE_ALLOW_SKIP=${ALLOW_SKIP}" \
  "CTX_AUTOMATION_REMOTE_AUTH_TEST_MODE=${CTX_AUTOMATION_REMOTE_AUTH_TEST_MODE:-key}" \
  "CTX_AUTOMATION_REMOTE_EXPECT_CONNECT_FAILURE=0" \
  "CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS=${REQUIRE_FIRST_TURN_SUCCESS}" \
  "CTX_REMOTE_BOOTSTRAP_CONTRACT_REPORT=${host_report}" \
  "CTX_REMOTE_BOOTSTRAP_FIRST_TURN_REPORT=${host_first_turn}" \
  pnpm -C "${ROOT}/core/apps/desktop" test:automation:remote-bootstrap
host_status=$?

container_status=0
if [[ "${RUN_CONTAINER}" == "1" ]]; then
  container_report="${CONTAINER_DIR}/contract-report.json"
  run_lane \
    "remote-container" \
    "${CONTAINER_DIR}" \
    "${container_report}" \
    env \
    "CTX_AUTOMATION_REMOTE_STRICT=${STRICT_REQUIRED}" \
    "CTX_AUTOMATION_REMOTE_ALLOW_SKIP=${ALLOW_SKIP}" \
    "CTX_AUTOMATION_SCENARIOS=${CTX_AUTOMATION_SCENARIOS:-remote-new-disk-isolated}" \
    "CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS=${REQUIRE_FIRST_TURN_SUCCESS}" \
    "CTX_REMOTE_CONTAINER_CONTRACT_REPORT=${container_report}" \
    pnpm -C "${ROOT}/core/apps/desktop" test:automation:remote-container-contract
  container_status=$?
fi

{
  echo "remote_contracts"
  echo "artifact_dir=${ARTIFACT_DIR}"
  echo "preflight=${PRECHECK_JSON}"
  echo "summary_tsv=${SUMMARY_TSV}"
  echo "host_status=${host_status}"
  echo "container_status=${container_status}"
  echo "run_container=${RUN_CONTAINER}"
  echo "strict_required=${STRICT_REQUIRED}"
  echo "allow_skip=${ALLOW_SKIP}"
  echo "dry_run=${DRY_RUN}"
} >"${SUMMARY_TXT}"

echo "[remote-contracts] summary: ${SUMMARY_TXT}" >&2
echo "[remote-contracts] tsv: ${SUMMARY_TSV}" >&2

if [[ "${host_status}" -ne 0 ]]; then
  exit "${host_status}"
fi
if [[ "${RUN_CONTAINER}" == "1" && "${container_status}" -ne 0 ]]; then
  exit "${container_status}"
fi

exit 0
