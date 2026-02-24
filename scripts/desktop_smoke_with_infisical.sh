#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/.. && pwd)"
CORE_DIR="${ROOT}/core"

SCENARIOS="${CTX_AUTOMATION_SCENARIOS:-local-import}"
INFISICAL_ENV="${INFISICAL_ENV:-dev}"
INFISICAL_PROJECT_ID="${INFISICAL_PROJECT_ID:-}"

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
  if [[ "${#ARGS[@]}" -gt 0 ]]; then
    exec infisical "${INFISICAL_RUN_ARGS[@]}" \
      env CTX_AUTOMATION_SCENARIOS="${SCENARIOS}" \
      pnpm -C apps/desktop exec wdio run automation/wdio.conf.cjs "${ARGS[@]}"
  fi
  exec infisical "${INFISICAL_RUN_ARGS[@]}" \
    env CTX_AUTOMATION_SCENARIOS="${SCENARIOS}" \
    pnpm -C apps/desktop exec wdio run automation/wdio.conf.cjs
fi

cd "${CORE_DIR}"
if [[ "${#ARGS[@]}" -gt 0 ]]; then
  exec env CTX_AUTOMATION_SCENARIOS="${SCENARIOS}" \
    pnpm -C apps/desktop exec wdio run automation/wdio.conf.cjs "${ARGS[@]}"
fi
exec env CTX_AUTOMATION_SCENARIOS="${SCENARIOS}" \
  pnpm -C apps/desktop exec wdio run automation/wdio.conf.cjs
