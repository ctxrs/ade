#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF' >&2
usage: check_avf_macos_host_prereqs.sh --restore-smoke MODE

Validates macOS host prerequisites for AVF save/restore coverage.
When restore smoke is skipped, the check is a no-op.
EOF
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

restore_smoke_mode=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --restore-smoke)
      [[ $# -ge 2 ]] || die "--restore-smoke requires a value"
      restore_smoke_mode="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      die "unknown argument: $1"
      ;;
  esac
done

[[ -n "$restore_smoke_mode" ]] || die "--restore-smoke is required"

case "$restore_smoke_mode" in
  skip)
    printf 'AVF macOS host preflight skipped because restore smoke is disabled\n'
    exit 0
    ;;
  required)
    ;;
  *)
    die "unsupported --restore-smoke mode: $restore_smoke_mode"
    ;;
esac

[[ "$(uname -s)" == "Darwin" ]] || die "AVF macOS host preflight only supports Darwin hosts"

if [[ -n "${CTX_AVF_MACOS_IOREG_ROOT_OVERRIDE:-}" ]]; then
  ioreg_text="$(cat "$CTX_AVF_MACOS_IOREG_ROOT_OVERRIDE")"
else
  ioreg_text="$(ioreg -n Root -d1 2>/dev/null || true)"
fi

[[ -n "$ioreg_text" ]] || die "failed to read IORegistry Root session state"

if [[ "$ioreg_text" != *'kCGSessionLoginDoneKey"=Yes'* ]] || [[ "$ioreg_text" != *'kCGSSessionOnConsoleKey"=Yes'* ]]; then
  die "AVF restore smoke requires an interactive macOS console login session"
fi

console_user="$(printf '%s\n' "$ioreg_text" | sed -n 's/.*kCGSSessionUserNameKey"="\([^"]*\)".*/\1/p' | head -n1)"

if [[ "$ioreg_text" == *'CGSSessionScreenIsLocked"=Yes'* ]]; then
  die "AVF restore smoke requires the macOS console session to remain unlocked; detected locked console session${console_user:+ for user ${console_user}}"
fi

printf 'AVF macOS host preflight ok: console session%s is logged in and unlocked\n' "${console_user:+ for user ${console_user}}"
