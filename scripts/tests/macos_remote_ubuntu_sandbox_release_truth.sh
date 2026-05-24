#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../.. && pwd)"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
ARTIFACT_DIR="${CTX_REMOTE_RELEASE_TRUTH_ARTIFACT_DIR:-${ROOT}/core/apps/desktop/automation/artifacts/remote-release-truth/${RUN_ID}}"
ARCHES_CSV="${CTX_UPDATER_E2E_ARCHES:-linux-x64,linux-arm64}"
RUN_CONTAINER="${CTX_REMOTE_RELEASE_TRUTH_RUN_CONTAINER:-1}"
REPORT_PATH="${ARTIFACT_DIR}/summary.json"
RUN_LOG="${ARTIFACT_DIR}/remote-release-truth.log"
REMOTE_REAL_SCRIPT="${ROOT}/core/apps/desktop/scripts/test_remote_real_ci.sh"
REMOTE_MATRIX_SCRIPT="${ROOT}/scripts/updater_e2e_remote_matrix.sh"

mkdir -p "${ARTIFACT_DIR}"

write_report() {
  local status="$1"
  local reason="$2"
  REPORT_PATH="${REPORT_PATH}" \
  REPORT_STATUS="${status}" \
  REPORT_REASON="${reason}" \
  REPORT_ARTIFACT_DIR="${ARTIFACT_DIR}" \
  REPORT_RUN_CONTAINER="${RUN_CONTAINER}" \
  node <<'NODE'
const fs = require("node:fs");
const path = require("node:path");

const payload = {
  schema_version: 1,
  status: process.env.REPORT_STATUS || "unknown",
  reason: process.env.REPORT_REASON || "",
  artifact_dir: process.env.REPORT_ARTIFACT_DIR || "",
  run_container: process.env.REPORT_RUN_CONTAINER === "1",
};

fs.mkdirSync(path.dirname(process.env.REPORT_PATH), { recursive: true });
fs.writeFileSync(process.env.REPORT_PATH, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
NODE
}

if [[ "$(uname -s)" != "Darwin" ]]; then
  write_report "skipped" "macos_only"
  echo "skip: mac remote release-truth lane only runs on macOS"
  exit 0
fi

if [[ -z "${CN_API_KEY:-}" ]]; then
  write_report "infra_unavailable" "missing_cn_api_key"
  echo "error: CN_API_KEY is required for the macOS remote release-truth lane" >&2
  exit 1
fi

if [[ -z "${OPENROUTER_API_KEY:-}" ]]; then
  write_report "infra_unavailable" "missing_openrouter_api_key"
  echo "error: OPENROUTER_API_KEY is required for the macOS remote release-truth lane" >&2
  exit 1
fi

if [[ ! -x "${REMOTE_REAL_SCRIPT}" ]]; then
  write_report "failed" "missing_remote_real_ci_wrapper"
  echo "error: missing remote real CI wrapper at ${REMOTE_REAL_SCRIPT}" >&2
  exit 1
fi

if [[ ! -x "${REMOTE_MATRIX_SCRIPT}" ]]; then
  write_report "failed" "missing_remote_matrix_wrapper"
  echo "error: missing remote matrix wrapper at ${REMOTE_MATRIX_SCRIPT}" >&2
  exit 1
fi

echo "[remote-release-truth] provisioning fresh Ubuntu host(s) and running Mac desktop remote contracts" >&2
set +e
env \
  CTX_REMOTE_CI_ARTIFACT_DIR="${ARTIFACT_DIR}" \
  CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS="${CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS:-0}" \
  CTX_AUTOMATION_REMOTE_STRICT=1 \
  CTX_AUTOMATION_REMOTE_ALLOW_SKIP=0 \
  CTX_AUTOMATION_CN_SHARED_BACKEND=0 \
  CTX_AUTOMATION_WDIO_LOG_LEVEL="${CTX_AUTOMATION_WDIO_LOG_LEVEL:-warn}" \
  "${REMOTE_MATRIX_SCRIPT}" run -- \
  "${REMOTE_REAL_SCRIPT}" \
  --run-host 1 \
  --run-container "${RUN_CONTAINER}" >"${RUN_LOG}" 2>&1
status=$?
set -e

set +e
node - "${ARTIFACT_DIR}" "${ARCHES_CSV}" "${RUN_CONTAINER}" <<'NODE'
const fs = require("node:fs");
const path = require("node:path");

const artifactDir = process.argv[2];
const arches = String(process.argv[3] || "")
  .split(",")
  .map((value) => value.trim())
  .filter(Boolean);
const runContainer = process.argv[4] === "1";

if (arches.length === 0) {
  console.error("no remote truth arches configured");
  process.exit(2);
}

const checks = [];
for (const arch of arches) {
  checks.push({
    path: path.join(artifactDir, arch, "remote-host", "contract-report.json"),
    expectedScope: "remote_host",
    expectedLane: "host",
  });
  if (runContainer) {
    checks.push({
      path: path.join(artifactDir, arch, "remote-container", "contract-report.json"),
      expectedScope: "remote_sandbox",
      expectedLane: "sandbox",
    });
  }
}

for (const check of checks) {
  if (!fs.existsSync(check.path)) {
    console.error(`missing report: ${check.path}`);
    process.exit(2);
  }
  const payload = JSON.parse(fs.readFileSync(check.path, "utf8"));
  const result = String(payload.result || "").trim().toLowerCase();
  if (result !== "passed" && result !== "pass") {
    console.error(`report did not pass: ${check.path} result=${result || "unknown"}`);
    process.exit(3);
  }
  const fixture = payload.fixture || {};
  if (String(fixture.proof_scope || "").trim() !== check.expectedScope) {
    console.error(
      `wrong proof scope for ${check.path}: expected ${check.expectedScope}, got ${String(fixture.proof_scope || "")}`,
    );
    process.exit(4);
  }
  if (String(fixture.host_mode || "").trim() !== "fresh-install") {
    console.error(
      `wrong host mode for ${check.path}: expected fresh-install, got ${String(fixture.host_mode || "")}`,
    );
    process.exit(5);
  }
  if (String(fixture.fixture_class || "").trim() === "docker-ssh") {
    console.error(`prepared docker fixture cannot satisfy release truth: ${check.path}`);
    process.exit(6);
  }
  if (String(fixture.lane || "").trim() !== check.expectedLane) {
    console.error(`wrong lane for ${check.path}: expected ${check.expectedLane}, got ${String(fixture.lane || "")}`);
    process.exit(7);
  }
}
NODE
validate_status=$?
set -e

if [[ "${status}" -eq 0 && "${validate_status}" -eq 0 ]]; then
  write_report "passed" "mac_desktop_remote_truth_passed"
  echo "[remote-release-truth] passed; report=${REPORT_PATH}" >&2
  exit 0
fi

failure_reason="remote_contract_failed"
if [[ "${validate_status}" -ne 0 ]]; then
  failure_reason="release_truth_guardrail_failed"
fi

write_report "failed" "${failure_reason}"
tail -n 200 "${RUN_LOG}" >&2 || true
echo "[remote-release-truth] failed; report=${REPORT_PATH}" >&2
if [[ "${status}" -ne 0 ]]; then
  exit "${status}"
fi
exit "${validate_status}"
