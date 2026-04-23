#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/.. && pwd)"
CORE_DIR="${ROOT}/core"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${CTX_SHARED_CARGO_TARGET_DIR:-}}"
eval "$(node "${CORE_DIR}/scripts/print_ctx_cache_env.cjs" --mode workspace --format shell --mkdir)"
VOLATILE_ROOT="${CTX_VOLATILE_ROOT}"
VOLATILE_TARGETS_DIR="${CTX_VOLATILE_TARGETS_DIR}"
VOLATILE_ARTIFACTS_DIR="${CTX_VOLATILE_ARTIFACTS_DIR}"
DEFAULT_CARGO_TARGET_DIR="${CARGO_TARGET_DIR}"

SCENARIOS="${CTX_AUTOMATION_SCENARIOS:-local-import}"
INFISICAL_ENV="${INFISICAL_ENV:-dev}"
INFISICAL_PROJECT_ID="${INFISICAL_PROJECT_ID:-}"
DEFAULT_AUTOMATION_TMP_BASE_DIR="${CTX_AUTOMATION_TMP_BASE_DIR:-${VOLATILE_ARTIFACTS_DIR}/ctx-desktop-e2e}"
mkdir -p "${DEFAULT_AUTOMATION_TMP_BASE_DIR}"
AUTOMATION_TMPDIR_CREATED=0
if [[ -n "${CTX_AUTOMATION_TMPDIR:-}" ]]; then
  AUTOMATION_TMPDIR="${CTX_AUTOMATION_TMPDIR}"
else
  AUTOMATION_TMPDIR="$(mktemp -d "${DEFAULT_AUTOMATION_TMP_BASE_DIR}/ctx-desktop-e2e-tmp.XXXXXX")"
  AUTOMATION_TMPDIR_CREATED=1
fi
mkdir -p "${AUTOMATION_TMPDIR}"
DEFAULT_CN_BACKEND_LOG="${AUTOMATION_TMPDIR}/crabnebula-backend.log"
DEFAULT_CN_DRIVER_LOG="${AUTOMATION_TMPDIR}/tauri-driver.log"
CTX_AUTOMATION_CN_BACKEND_LOG="${CTX_AUTOMATION_CN_BACKEND_LOG:-${DEFAULT_CN_BACKEND_LOG}}"
CTX_AUTOMATION_CN_DRIVER_LOG="${CTX_AUTOMATION_CN_DRIVER_LOG:-${DEFAULT_CN_DRIVER_LOG}}"
mkdir -p \
  "${DEFAULT_CARGO_TARGET_DIR}" \
  "${DEFAULT_CARGO_TARGET_DIR}/debug" \
  "${DEFAULT_CARGO_TARGET_DIR}/debug/deps" \
  "${DEFAULT_CARGO_TARGET_DIR}/debug/.fingerprint" \
  "${DEFAULT_CARGO_TARGET_DIR}/release" \
  "${DEFAULT_CARGO_TARGET_DIR}/release/deps" \
  "${DEFAULT_CARGO_TARGET_DIR}/release/.fingerprint"

ensure_desktop_automation_deps() {
  if [[ -d node_modules ]] \
    && pnpm -C apps/web exec which vite >/dev/null 2>&1 \
    && pnpm -C apps/desktop exec which wdio >/dev/null 2>&1; then
    return
  fi
  echo "[desktop-smoke] repairing missing core/apps/web/apps/desktop automation toolchain" >&2
  pnpm install --frozen-lockfile >/dev/null
  pnpm -C apps/web install --frozen-lockfile >/dev/null
  pnpm -C apps/desktop install --frozen-lockfile >/dev/null
  [[ -d node_modules ]]
  pnpm -C apps/web exec which vite >/dev/null
  pnpm -C apps/desktop exec which wdio >/dev/null
}

cleanup_automation_tmpdir() {
  if [[ "${AUTOMATION_TMPDIR_CREATED}" != "1" ]]; then
    return
  fi
  if [[ "$(uname -s)" == "Linux" ]] && command -v findmnt >/dev/null 2>&1; then
    findmnt -rn -o TARGET 2>/dev/null | sort -r | while IFS= read -r mount_target; do
      if [[ "${mount_target}" != "${AUTOMATION_TMPDIR}" && "${mount_target}" != "${AUTOMATION_TMPDIR}/"* ]]; then
        continue
      fi
      echo "[desktop-smoke] unmounting automation tmpdir mount ${mount_target}" >&2
      if command -v fusermount3 >/dev/null 2>&1; then
        fusermount3 -u "${mount_target}" >/dev/null 2>&1 || true
      fi
      if findmnt -rn --target "${mount_target}" >/dev/null 2>&1; then
        umount -l "${mount_target}" >/dev/null 2>&1 || true
      fi
    done
  fi
  if [[ "${CTX_AUTOMATION_KEEP_TMPDIR:-0}" == "1" ]]; then
    echo "[desktop-smoke] preserving automation tmpdir ${AUTOMATION_TMPDIR}" >&2
    return
  fi
  rm -rf "${AUTOMATION_TMPDIR}"
}

trap cleanup_automation_tmpdir EXIT

run_wdio() {
  local status=0
  "$@" || status=$?
  if [[ "${status}" -ne 0 ]]; then
    echo "[desktop-smoke] CrabNebula backend log: ${CTX_AUTOMATION_CN_BACKEND_LOG}" >&2
    echo "[desktop-smoke] CrabNebula driver log: ${CTX_AUTOMATION_CN_DRIVER_LOG}" >&2
  fi
  return "${status}"
}

# Accept optional leading `--` from package scripts and pass through remaining args.
ARGS=("$@")
if [[ "${#ARGS[@]}" -gt 0 && "${ARGS[0]}" == "--" ]]; then
  ARGS=("${ARGS[@]:1}")
fi

INFISICAL_HELP_BYPASS=0
for arg in "${ARGS[@]}"; do
  case "${arg}" in
    -h|--help|--list) INFISICAL_HELP_BYPASS=1 ;;
  esac
done

if [[ "$(uname -s)" == "Darwin" && -z "${CN_API_KEY:-}" && "${INFISICAL_HELP_BYPASS}" != "1" ]]; then
  if ! command -v infisical >/dev/null 2>&1; then
    echo "error: CN_API_KEY is required for macOS desktop automation." >&2
    echo "hint: install Infisical CLI and run from core/.infisical.json context, or export CN_API_KEY manually." >&2
    exit 1
  fi
  if [[ ! -f "${CORE_DIR}/.infisical.json" ]]; then
    echo "error: missing ${CORE_DIR}/.infisical.json required to load CN_API_KEY from Infisical." >&2
    exit 1
  fi
  cd "${CORE_DIR}"
  : "${INFISICAL_PROJECT_ID:?Set INFISICAL_PROJECT_ID to load automation credentials from Infisical}"
  INFISICAL_RUN_ARGS=(run --env "${INFISICAL_ENV}" --projectId "${INFISICAL_PROJECT_ID}" --)
  ensure_desktop_automation_deps
	  if [[ "${#ARGS[@]}" -gt 0 ]]; then
	    run_wdio infisical "${INFISICAL_RUN_ARGS[@]}" \
	      env CTX_AUTOMATION_SCENARIOS="${SCENARIOS}" \
	      CARGO_TARGET_DIR="${DEFAULT_CARGO_TARGET_DIR}" \
	      TMPDIR="${AUTOMATION_TMPDIR}" \
	      TMP="${AUTOMATION_TMPDIR}" \
	      TEMP="${AUTOMATION_TMPDIR}" \
	      CTX_AUTOMATION_TMPDIR="${AUTOMATION_TMPDIR}" \
	      CTX_AUTOMATION_CN_BACKEND_LOG="${CTX_AUTOMATION_CN_BACKEND_LOG}" \
	      CTX_AUTOMATION_CN_DRIVER_LOG="${CTX_AUTOMATION_CN_DRIVER_LOG}" \
	      pnpm -C apps/desktop exec wdio run automation/wdio.conf.cjs "${ARGS[@]}"
	    exit $?
  fi
	  run_wdio infisical "${INFISICAL_RUN_ARGS[@]}" \
	    env CTX_AUTOMATION_SCENARIOS="${SCENARIOS}" \
	    CARGO_TARGET_DIR="${DEFAULT_CARGO_TARGET_DIR}" \
	    TMPDIR="${AUTOMATION_TMPDIR}" \
	    TMP="${AUTOMATION_TMPDIR}" \
	    TEMP="${AUTOMATION_TMPDIR}" \
	    CTX_AUTOMATION_TMPDIR="${AUTOMATION_TMPDIR}" \
	    CTX_AUTOMATION_CN_BACKEND_LOG="${CTX_AUTOMATION_CN_BACKEND_LOG}" \
	    CTX_AUTOMATION_CN_DRIVER_LOG="${CTX_AUTOMATION_CN_DRIVER_LOG}" \
	    pnpm -C apps/desktop exec wdio run automation/wdio.conf.cjs
  exit $?
fi

cd "${CORE_DIR}"
ensure_desktop_automation_deps
if [[ "${#ARGS[@]}" -gt 0 ]]; then
	run_wdio env CTX_AUTOMATION_SCENARIOS="${SCENARIOS}" \
	  CARGO_TARGET_DIR="${DEFAULT_CARGO_TARGET_DIR}" \
	  TMPDIR="${AUTOMATION_TMPDIR}" \
	  TMP="${AUTOMATION_TMPDIR}" \
	  TEMP="${AUTOMATION_TMPDIR}" \
	  CTX_AUTOMATION_TMPDIR="${AUTOMATION_TMPDIR}" \
	  CTX_AUTOMATION_CN_BACKEND_LOG="${CTX_AUTOMATION_CN_BACKEND_LOG}" \
	  CTX_AUTOMATION_CN_DRIVER_LOG="${CTX_AUTOMATION_CN_DRIVER_LOG}" \
	  pnpm -C apps/desktop exec wdio run automation/wdio.conf.cjs "${ARGS[@]}"
  exit $?
fi
run_wdio env CTX_AUTOMATION_SCENARIOS="${SCENARIOS}" \
  CARGO_TARGET_DIR="${DEFAULT_CARGO_TARGET_DIR}" \
  TMPDIR="${AUTOMATION_TMPDIR}" \
  TMP="${AUTOMATION_TMPDIR}" \
  TEMP="${AUTOMATION_TMPDIR}" \
  CTX_AUTOMATION_TMPDIR="${AUTOMATION_TMPDIR}" \
  CTX_AUTOMATION_CN_BACKEND_LOG="${CTX_AUTOMATION_CN_BACKEND_LOG}" \
  CTX_AUTOMATION_CN_DRIVER_LOG="${CTX_AUTOMATION_CN_DRIVER_LOG}" \
  pnpm -C apps/desktop exec wdio run automation/wdio.conf.cjs
exit $?
