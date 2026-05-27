#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../../../.. && pwd)"
FIXTURE="${CTX_HARNESS_INSTALL_MATRIX_FIXTURE:-${ROOT}/core/apps/desktop/automation/fixtures/harness_install_matrix.json}"
SMOKE_SCRIPT="${CTX_HARNESS_INSTALL_MATRIX_SMOKE_SCRIPT:-${ROOT}/scripts/release_runtime_install_smoke.sh}"
DAEMON_BIN="${CTX_HARNESS_INSTALL_MATRIX_DAEMON_BIN:-${CTX_DAEMON_BIN:-}}"
APP_PATH="${CTX_HARNESS_INSTALL_MATRIX_APP:-${CTX_DESKTOP_APP:-}}"
BUNDLE_DIR="${CTX_HARNESS_INSTALL_MATRIX_BUNDLE_DIR:-${CTX_BUNDLE_DIR:-}}"
TIMEOUT_SECONDS="${CTX_HARNESS_INSTALL_MATRIX_TIMEOUT_SECONDS:-900}"
RELEASE_PLAN_PATH="${CTX_HARNESS_INSTALL_MATRIX_RELEASE_PLAN:-}"

usage() {
  cat <<'USAGE'
Usage:
  run_harness_install_matrix.sh [--lane preview|release|nightly|all] [--platform auto|macos|linux|all] [--target host|sandbox|all] [--provider ID[,ID...]] [--cell ID[,ID...]] [--artifacts-dir DIR] [--list] [--dry-run] [--fail-fast]

Options:
  --lane VALUE           Matrix lane to run. Default: release.
  --platform VALUE       Platform filter. Default: auto (current OS).
  --target VALUE         Execution target filter. Default: all.
  --provider ID[,ID...]  Provider filter. Can be repeated.
  --cell ID[,ID...]      Exact cell filter. Can be repeated.
  --artifacts-dir DIR    Output root for per-cell logs and summary.
  --list                 Print selected matrix cells and exit.
  --dry-run              Print resolved commands without running them.
  --fail-fast            Stop on first failed cell. Default is to run all selected cells and report all failures.

Live execution requires:
  CTX_HARNESS_INSTALL_MATRIX_DAEMON_BIN or CTX_DAEMON_BIN
  either CTX_HARNESS_INSTALL_MATRIX_APP/CTX_DESKTOP_APP or CTX_HARNESS_INSTALL_MATRIX_BUNDLE_DIR/CTX_BUNDLE_DIR
USAGE
}

current_platform() {
  case "$(uname -s)" in
    Darwin) printf "macos" ;;
    Linux) printf "linux" ;;
    *)
      echo "error: unsupported host platform: $(uname -s)" >&2
      exit 1
      ;;
  esac
}

LANE="release"
PLATFORM="${CTX_HARNESS_INSTALL_MATRIX_PLATFORM:-auto}"
TARGET="all"
LIST_ONLY=0
DRY_RUN=0
FAIL_FAST=0
ARTIFACTS_DIR="${CTX_HARNESS_INSTALL_MATRIX_ARTIFACTS_DIR:-}"
declare -a REQUESTED_PROVIDERS=()
declare -a REQUESTED_CELLS=()

source_sha() {
  git -C "${ROOT}" rev-parse HEAD
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --lane)
      LANE="$(printf "%s" "${2:-}" | tr '[:upper:]' '[:lower:]')"
      shift 2
      ;;
    --platform)
      PLATFORM="$(printf "%s" "${2:-}" | tr '[:upper:]' '[:lower:]')"
      shift 2
      ;;
    --target)
      TARGET="$(printf "%s" "${2:-}" | tr '[:upper:]' '[:lower:]')"
      shift 2
      ;;
    --provider)
      IFS=',' read -r -a values <<<"${2:-}"
      for value in "${values[@]}"; do
        trimmed="$(printf "%s" "$value" | tr -d '[:space:]')"
        [[ -n "$trimmed" ]] && REQUESTED_PROVIDERS+=("$trimmed")
      done
      shift 2
      ;;
    --cell)
      IFS=',' read -r -a values <<<"${2:-}"
      for value in "${values[@]}"; do
        trimmed="$(printf "%s" "$value" | tr -d '[:space:]')"
        [[ -n "$trimmed" ]] && REQUESTED_CELLS+=("$trimmed")
      done
      shift 2
      ;;
    --artifacts-dir)
      ARTIFACTS_DIR="${2:-}"
      shift 2
      ;;
    --list)
      LIST_ONLY=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --fail-fast)
      FAIL_FAST=1
      shift
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
  preview|release|nightly|all) ;;
  *)
    echo "error: --lane must be preview|release|nightly|all (got '${LANE}')" >&2
    exit 1
    ;;
esac

case "${PLATFORM}" in
  auto) PLATFORM="$(current_platform)" ;;
  macos|linux|all) ;;
  *)
    echo "error: --platform must be auto|macos|linux|all (got '${PLATFORM}')" >&2
    exit 1
    ;;
esac

case "${TARGET}" in
  host|sandbox|all) ;;
  *)
    echo "error: --target must be host|sandbox|all (got '${TARGET}')" >&2
    exit 1
    ;;
esac

if [[ -n "${RELEASE_PLAN_PATH}" ]]; then
  if [[ ! -f "${RELEASE_PLAN_PATH}" ]]; then
    echo "error: resolved release plan not found: ${RELEASE_PLAN_PATH}" >&2
    exit 1
  fi
  node - "${RELEASE_PLAN_PATH}" "$(source_sha)" <<'NODE'
const fs = require("node:fs");
const [releasePlanPath, expectedSourceCommit] = process.argv.slice(2);
const plan = JSON.parse(fs.readFileSync(releasePlanPath, "utf8"));
if (String(plan.source_commit || "").trim() !== expectedSourceCommit) {
  console.error(`error: resolved release plan source_commit mismatch: expected ${expectedSourceCommit}, got ${plan.source_commit || "<missing>"}`);
  process.exit(1);
}
NODE
fi

if [[ ! -f "${FIXTURE}" ]]; then
  echo "error: harness install matrix fixture not found: ${FIXTURE}" >&2
  exit 1
fi
if [[ ! -x "${SMOKE_SCRIPT}" ]]; then
  echo "error: release runtime install smoke script not executable: ${SMOKE_SCRIPT}" >&2
  exit 1
fi
if ! [[ "${TIMEOUT_SECONDS}" =~ ^[0-9]+$ ]] || [[ "${TIMEOUT_SECONDS}" -le 0 ]]; then
  echo "error: CTX_HARNESS_INSTALL_MATRIX_TIMEOUT_SECONDS must be a positive integer" >&2
  exit 1
fi

if [[ -z "${ARTIFACTS_DIR}" ]]; then
  ARTIFACTS_DIR="${ROOT}/core/apps/desktop/automation/artifacts/harness-install-matrix/$(date -u +%Y%m%dT%H%M%SZ)"
fi

providers_csv="$(IFS=,; printf "%s" "${REQUESTED_PROVIDERS[*]:-}")"
cells_csv="$(IFS=,; printf "%s" "${REQUESTED_CELLS[*]:-}")"

selected_json="$(
  FIXTURE="${FIXTURE}" \
  LANE="${LANE}" \
  PLATFORM="${PLATFORM}" \
  TARGET="${TARGET}" \
  PROVIDERS_CSV="${providers_csv}" \
  CELLS_CSV="${cells_csv}" \
  node <<'NODE'
const fs = require("node:fs");
const fixture = JSON.parse(fs.readFileSync(process.env.FIXTURE, "utf8"));
const wantedLane = process.env.LANE;
const wantedPlatform = process.env.PLATFORM;
const wantedTarget = process.env.TARGET;
const providers = new Set(String(process.env.PROVIDERS_CSV || "").split(",").map((value) => value.trim()).filter(Boolean));
const cells = new Set(String(process.env.CELLS_CSV || "").split(",").map((value) => value.trim()).filter(Boolean));
const platformTargets = new Map((fixture.platform_targets || []).map((target) => [target.id, target]));
const rows = [];

for (const provider of fixture.providers || []) {
  for (const [targetId, cell] of Object.entries(provider.cells || {})) {
    const target = platformTargets.get(targetId) || {};
    const id = `${provider.id}.${targetId}`;
    const lanes = Array.isArray(cell.lanes) ? cell.lanes : [];
    if (cell.support !== "supported") continue;
    if (wantedLane !== "all" && !lanes.includes(wantedLane)) continue;
    if (wantedPlatform !== "all" && target.platform !== wantedPlatform) continue;
    if (wantedTarget !== "all" && target.execution_target !== wantedTarget) continue;
    if (providers.size > 0 && !providers.has(provider.id)) continue;
    if (cells.size > 0 && !cells.has(id)) continue;
    rows.push({
      id,
      provider_id: provider.id,
      platform: target.platform,
      execution_target: target.execution_target,
      install_target: cell.install_target || target.install_target,
      runner_script: (cell.runner || {}).script || "scripts/release_runtime_install_smoke.sh",
    });
  }
}

rows.sort((a, b) => a.id.localeCompare(b.id));
const matchedProviders = new Set(rows.map((row) => row.provider_id));
const matchedCells = new Set(rows.map((row) => row.id));
const unmatchedProviders = [...providers].filter((providerId) => !matchedProviders.has(providerId));
const unmatchedCells = [...cells].filter((cellId) => !matchedCells.has(cellId));
if (unmatchedProviders.length > 0 || unmatchedCells.length > 0) {
  for (const providerId of unmatchedProviders) {
    console.error(`error: requested provider selected no supported cells after filters: ${providerId}`);
  }
  for (const cellId of unmatchedCells) {
    console.error(`error: requested cell selected no supported cells after filters: ${cellId}`);
  }
  process.exit(1);
}
process.stdout.write(`${JSON.stringify(rows)}\n`);
NODE
)"

selected_count="$(SELECTED_JSON="${selected_json}" node -e 'const rows=JSON.parse(process.env.SELECTED_JSON); process.stdout.write(String(rows.length));')"
if [[ "${selected_count}" -eq 0 ]]; then
  echo "error: no harness install matrix cells selected" >&2
  exit 1
fi

if [[ "${LIST_ONLY}" -eq 1 ]]; then
  SELECTED_JSON="${selected_json}" node <<'NODE'
for (const row of JSON.parse(process.env.SELECTED_JSON)) {
  console.log(`${row.id}\tprovider=${row.provider_id}\tplatform=${row.platform}\ttarget=${row.execution_target}\tinstall_target=${row.install_target}`);
}
NODE
  exit 0
fi

if [[ "${DRY_RUN}" -ne 1 ]]; then
  if [[ -z "${DAEMON_BIN}" ]]; then
    echo "error: CTX_HARNESS_INSTALL_MATRIX_DAEMON_BIN or CTX_DAEMON_BIN is required for live execution" >&2
    exit 1
  fi
  if [[ -z "${APP_PATH}" && -z "${BUNDLE_DIR}" ]]; then
    echo "error: CTX_HARNESS_INSTALL_MATRIX_APP/CTX_DESKTOP_APP or CTX_HARNESS_INSTALL_MATRIX_BUNDLE_DIR/CTX_BUNDLE_DIR is required for live execution" >&2
    exit 1
  fi
fi

mkdir -p "${ARTIFACTS_DIR}"
summary_path="${ARTIFACTS_DIR}/summary.jsonl"
evidence_path="${ARTIFACTS_DIR}/harness-install-evidence.json"
: >"${summary_path}"
failures=0

run_cell() {
  local row_json="$1"
  local cell_id provider_id install_target cell_dir stdout_log stderr_log status_path
  cell_id="$(ROW_JSON="${row_json}" node -e 'const row=JSON.parse(process.env.ROW_JSON); process.stdout.write(row.id)')"
  provider_id="$(ROW_JSON="${row_json}" node -e 'const row=JSON.parse(process.env.ROW_JSON); process.stdout.write(row.provider_id)')"
  install_target="$(ROW_JSON="${row_json}" node -e 'const row=JSON.parse(process.env.ROW_JSON); process.stdout.write(row.install_target)')"
  cell_dir="${ARTIFACTS_DIR}/${cell_id//[^a-zA-Z0-9._-]/_}"
  stdout_log="${cell_dir}/stdout.log"
  stderr_log="${cell_dir}/stderr.log"
  status_path="${cell_dir}/status.json"
  mkdir -p "${cell_dir}"

  local -a cmd=(
    "${SMOKE_SCRIPT}"
    --daemon-bin "${DAEMON_BIN:-/dev/null}"
    --provider "${provider_id}"
    --target "${install_target}"
    --complete
    --timeout-seconds "${TIMEOUT_SECONDS}"
  )
  if [[ -n "${APP_PATH}" ]]; then
    cmd+=(--app "${APP_PATH}")
  else
    cmd+=(--bundle-dir "${BUNDLE_DIR:-/dev/null}")
  fi

  if [[ "${DRY_RUN}" -eq 1 ]]; then
    printf 'dry-run %s:' "${cell_id}"
    printf ' %q' "${cmd[@]}"
    printf '\n'
    ROW_JSON="${row_json}" STATUS="dry-run" EXIT_CODE="0" STATUS_PATH="${status_path}" SUMMARY_PATH="${summary_path}" node <<'NODE'
const fs = require("node:fs");
const row = JSON.parse(process.env.ROW_JSON);
const payload = { ...row, status: process.env.STATUS, exit_code: Number(process.env.EXIT_CODE || 0) };
fs.writeFileSync(process.env.STATUS_PATH, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
fs.appendFileSync(process.env.SUMMARY_PATH, `${JSON.stringify(payload)}\n`, "utf8");
NODE
    return 0
  fi

  echo "harness-install: running ${cell_id}" >&2
  local exit_code=0
  if "${cmd[@]}" >"${stdout_log}" 2>"${stderr_log}"; then
    exit_code=0
  else
    exit_code=$?
  fi

  local status="passed"
  if [[ "${exit_code}" -ne 0 ]]; then
    status="failed"
  fi
  ROW_JSON="${row_json}" STATUS="${status}" EXIT_CODE="${exit_code}" STDOUT_LOG="${stdout_log}" STDERR_LOG="${stderr_log}" STATUS_PATH="${status_path}" SUMMARY_PATH="${summary_path}" node <<'NODE'
const fs = require("node:fs");
const row = JSON.parse(process.env.ROW_JSON);
const payload = {
  ...row,
  status: process.env.STATUS,
  exit_code: Number(process.env.EXIT_CODE || 0),
  logs: {
    stdout: process.env.STDOUT_LOG,
    stderr: process.env.STDERR_LOG,
  },
};
fs.writeFileSync(process.env.STATUS_PATH, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
fs.appendFileSync(process.env.SUMMARY_PATH, `${JSON.stringify(payload)}\n`, "utf8");
NODE
  return "${exit_code}"
}

write_evidence() {
  SELECTED_JSON="${selected_json}" \
  SUMMARY_PATH="${summary_path}" \
  EVIDENCE_PATH="${evidence_path}" \
  FIXTURE="${FIXTURE}" \
  LANE="${LANE}" \
  PLATFORM="${PLATFORM}" \
  TARGET="${TARGET}" \
  ARTIFACTS_DIR="${ARTIFACTS_DIR}" \
  RELEASE_PLAN_PATH="${RELEASE_PLAN_PATH}" \
  DRY_RUN="${DRY_RUN}" \
  FAILURES="${failures}" \
  ROOT="${ROOT}" \
  node <<'NODE'
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const childProcess = require("node:child_process");
const { buildReleasePlanEvidenceDigest } = require(`${process.env.ROOT}/core/scripts/lib/release_evidence_validation.cjs`);

const readFile = (filePath) => fs.existsSync(filePath) ? fs.readFileSync(filePath, "utf8") : "";
const sha256File = (filePath) => crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
const sourceSha = (() => {
  try {
    return childProcess.execFileSync("git", ["-C", process.env.ROOT, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
  } catch {
    return "";
  }
})();
const results = readFile(process.env.SUMMARY_PATH)
  .split(/\r?\n/u)
  .map((line) => line.trim())
  .filter(Boolean)
  .map((line) => JSON.parse(line));
const statusCounts = results.reduce((acc, row) => {
  acc[row.status] = (acc[row.status] || 0) + 1;
  return acc;
}, {});
const releasePlanPath = String(process.env.RELEASE_PLAN_PATH || "").trim();
const releasePlan = releasePlanPath
  ? JSON.parse(fs.readFileSync(releasePlanPath, "utf8"))
  : null;
if (releasePlan && String(releasePlan.source_commit || "").trim() !== sourceSha) {
  throw new Error(`resolved release plan source_commit mismatch: expected ${sourceSha}, got ${releasePlan.source_commit || "<missing>"}`);
}
const releasePlanDigest = releasePlan ? buildReleasePlanEvidenceDigest(releasePlan) : "";
const payload = {
  schema_version: 1,
  kind: "ctx.harness_install_matrix_evidence.v1",
  source_sha: sourceSha,
  generated_at: new Date().toISOString(),
  matrix: {
    path: process.env.FIXTURE,
    sha256: sha256File(process.env.FIXTURE),
  },
  selection: {
    lane: process.env.LANE,
    platform: process.env.PLATFORM,
    target: process.env.TARGET,
    dry_run: process.env.DRY_RUN === "1",
  },
  host: {
    os_type: os.type(),
    platform: os.platform(),
    arch: os.arch(),
    release: os.release(),
    machine: os.machine ? os.machine() : "",
  },
  artifacts_dir: process.env.ARTIFACTS_DIR,
  release_plan_digest: releasePlanDigest || undefined,
  release_plan_id: releasePlanDigest ? `release-plan-sha256:${releasePlanDigest}` : undefined,
  selected_cells: JSON.parse(process.env.SELECTED_JSON),
  result_counts: statusCounts,
  failed_cells: results.filter((row) => row.status === "failed").map((row) => row.id),
  results,
};
fs.writeFileSync(process.env.EVIDENCE_PATH, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
NODE
}

while IFS= read -r row_json; do
  [[ -z "${row_json}" ]] && continue
  if run_cell "${row_json}"; then
    :
  else
    failures=$((failures + 1))
    if [[ "${FAIL_FAST}" -eq 1 ]]; then
      break
    fi
  fi
done < <(SELECTED_JSON="${selected_json}" node -e 'for (const row of JSON.parse(process.env.SELECTED_JSON)) console.log(JSON.stringify(row));')

write_evidence

if [[ "${failures}" -gt 0 ]]; then
  echo "error: harness install matrix failed ${failures} cell(s); summary: ${summary_path}; evidence: ${evidence_path}" >&2
  exit 1
fi

echo "ok: harness install matrix completed ${selected_count} cell(s); summary: ${summary_path}; evidence: ${evidence_path}"
