#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../../../.. && pwd)"
ARTIFACT_DIR="${CTX_REMOTE_CI_ARTIFACT_DIR:-$(mktemp -d /tmp/ctx-remote-real-ci.XXXXXX)}"
RUN_CONTAINER="${CTX_REMOTE_CI_RUN_CONTAINER:-1}"

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  cat <<'USAGE'
usage:
  test_remote_real_ci.sh

Runs the remote host bootstrap contract and (optionally) remote container contract
against the currently configured remote target env.

important env:
  CTX_REMOTE_CI_ARTIFACT_DIR
  CTX_REMOTE_CI_RUN_CONTAINER=1|0
  CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS=1|0
USAGE
  exit 0
fi

mkdir -p "${ARTIFACT_DIR}"

echo "[remote-real-ci] artifact dir: ${ARTIFACT_DIR}" >&2

host_log="${ARTIFACT_DIR}/remote-host.log"
container_log="${ARTIFACT_DIR}/remote-container.log"
summary_path="${ARTIFACT_DIR}/remote-real-ci-summary.txt"

host_status=1
container_status=0

set +e
CTX_AUTOMATION_REMOTE_AUTH_TEST_MODE="${CTX_AUTOMATION_REMOTE_AUTH_TEST_MODE:-key}" \
CTX_AUTOMATION_REMOTE_EXPECT_CONNECT_FAILURE=0 \
CTX_REMOTE_BOOTSTRAP_FIRST_TURN_REPORT="${ARTIFACT_DIR}/remote-host-first-turn.json" \
CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS="${CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS:-0}" \
pnpm -C "${ROOT}/core/apps/desktop" test:automation:remote-bootstrap >"${host_log}" 2>&1
host_status=$?
set -e

if [[ "${RUN_CONTAINER}" == "1" ]]; then
  container_report="${ARTIFACT_DIR}/remote-container-contract.json"
  set +e
  CTX_AUTOMATION_SCENARIOS="${CTX_AUTOMATION_SCENARIOS:-remote-new-disk-isolated}" \
  CTX_REMOTE_CONTAINER_CONTRACT_REPORT="${container_report}" \
  CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS="${CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS:-0}" \
  pnpm -C "${ROOT}/core/apps/desktop" test:automation --spec automation/specs/remote-container-contract.spec.cjs >"${container_log}" 2>&1
  container_status=$?
  set -e
  if [[ ${container_status} -eq 0 && -f "${container_report}" ]]; then
    if node -e 'const fs=require("fs"); const p=process.argv[1]; const raw=String(fs.readFileSync(p,"utf8")||"{}"); const v=JSON.parse(raw); process.exit(v && v.skipped ? 0 : 1);' "${container_report}"; then
      echo "[remote-real-ci] remote container lane was skipped; treating as infra_unavailable (see ${container_report})" >&2
      container_status=86
    fi
  fi
fi

{
  echo "remote_real_ci"
  echo "artifact_dir=${ARTIFACT_DIR}"
  echo "host_status=${host_status}"
  echo "container_status=${container_status}"
  echo "run_container=${RUN_CONTAINER}"
  echo "host_log=${host_log}"
  if [[ "${RUN_CONTAINER}" == "1" ]]; then
    echo "container_log=${container_log}"
  fi
} >"${summary_path}"

if [[ ${host_status} -ne 0 ]]; then
  echo "[remote-real-ci] remote host contract failed; see ${host_log}" >&2
  exit ${host_status}
fi
if [[ "${RUN_CONTAINER}" == "1" && ${container_status} -ne 0 ]]; then
  echo "[remote-real-ci] remote container contract failed; see ${container_log}" >&2
  exit ${container_status}
fi

echo "[remote-real-ci] complete" >&2
