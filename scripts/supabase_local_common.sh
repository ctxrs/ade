#!/usr/bin/env bash

supabase_repo_root() {
  cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd
}

supabase_cd_repo_root() {
  local root
  root="$(supabase_repo_root)"
  cd "$root"
}

supabase_require_cli() {
  if ! command -v supabase >/dev/null 2>&1; then
    echo "error: supabase CLI not found. Install: https://supabase.com/docs/guides/cli" >&2
    exit 1
  fi
}

supabase_warn_missing_cli() {
  if ! command -v supabase >/dev/null 2>&1; then
    echo "warning: supabase CLI not found; skipping local Supabase start." >&2
    exit 0
  fi
}

supabase_require_ctx_local_project() {
  if ! grep -q '^project_id = "ctx-local"$' supabase/config.toml; then
    echo "error: expected supabase/config.toml to declare project_id = \"ctx-local\"." >&2
    echo "Run local Supabase commands from the repository root, not with --workdir supabase." >&2
    exit 1
  fi
}

supabase_reject_wrong_workdir_stack() {
  if ! command -v docker >/dev/null 2>&1; then
    return 0
  fi

  local running_containers
  running_containers="$(docker ps --format '{{.Names}}' 2>/dev/null || true)"
  if printf '%s\n' "$running_containers" | grep -qx 'supabase_db_supabase'; then
    echo "error: found a running local Supabase stack for project 'supabase'." >&2
    echo "Stop it with: supabase stop --workdir supabase" >&2
    echo "Then use the repo-root scripts, which target project_id 'ctx-local'." >&2
    exit 1
  fi
}

supabase_prepare_local_project() {
  supabase_cd_repo_root
  supabase_require_ctx_local_project
  supabase_reject_wrong_workdir_stack
}
