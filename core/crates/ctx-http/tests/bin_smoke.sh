#!/usr/bin/env bash

set -euo pipefail

ctx_bin="$1"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT
ctx_copy="$tmpdir/ctx"
cp "$ctx_bin" "$ctx_copy"
chmod +x "$ctx_copy"

root_help="$("$ctx_copy" --help)"
printf '%s\n' "$root_help" | grep -F "ctx daemon and CLI" >/dev/null
printf '%s\n' "$root_help" | grep -F "serve" >/dev/null
printf '%s\n' "$root_help" | grep -F "init" >/dev/null
printf '%s\n' "$root_help" | grep -F "self-update" >/dev/null

serve_help="$("$ctx_copy" serve --help)"
printf '%s\n' "$serve_help" | grep -F -- "--bind" >/dev/null
printf '%s\n' "$serve_help" | grep -F -- "--data-dir" >/dev/null

init_help="$("$ctx_copy" init --help)"
printf '%s\n' "$init_help" | grep -F -- "--root" >/dev/null

self_update_help="$("$ctx_copy" self-update --help)"
printf '%s\n' "$self_update_help" | grep -F -- "--channel" >/dev/null
printf '%s\n' "$self_update_help" | grep -F -- "--check" >/dev/null
printf '%s\n' "$self_update_help" | grep -F -- "--yes" >/dev/null
