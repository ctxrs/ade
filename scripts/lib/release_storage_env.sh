#!/usr/bin/env bash

release_storage_provider() {
  local provider="${RELEASE_STORAGE_PROVIDER:-${CTX_RELEASE_STORAGE_PROVIDER:-${CTX_RELEASE_STORAGE_BACKEND:-r2}}}"
  printf '%s' "$provider" | tr 'A-Z' 'a-z'
}

release_storage_bucket() {
  local provider
  provider="$(release_storage_provider)"
  if [[ "$provider" == "r2" ]]; then
    printf '%s' "${RELEASE_STORAGE_BUCKET:-${CTX_RELEASES_R2_BUCKET:-${CTX_RELEASE_R2_BUCKET:-${RELEASE_R2_BUCKET:-}}}}"
    return 0
  fi
  printf '%s' "${RELEASE_STORAGE_BUCKET:-${SUPABASE_STORAGE_BUCKET:-}}"
}

release_storage_public_origin() {
  local origin="${RELEASE_PUBLIC_STORAGE_ORIGIN:-${SUPABASE_PUBLIC_URL:-https://api.ctx.rs}}"
  printf '%s' "${origin%/}"
}

release_storage_r2_endpoint() {
  if [[ -n "${RELEASE_R2_ENDPOINT:-${CTX_RELEASE_R2_ENDPOINT:-}}" ]]; then
    printf '%s' "${RELEASE_R2_ENDPOINT:-${CTX_RELEASE_R2_ENDPOINT:-}}"
    return 0
  fi
  if [[ -n "${RELEASE_R2_ACCOUNT_ID:-${CTX_RELEASE_R2_ACCOUNT_ID:-}}" ]]; then
    printf 'https://%s.r2.cloudflarestorage.com' "${RELEASE_R2_ACCOUNT_ID:-${CTX_RELEASE_R2_ACCOUNT_ID:-}}"
  fi
}

release_storage_has_write_env() {
  local provider bucket
  provider="$(release_storage_provider)"
  bucket="$(release_storage_bucket)"
  case "$provider" in
    supabase)
      [[ -n "${SUPABASE_URL:-}" && -n "${SUPABASE_SERVICE_ROLE_KEY:-}" && -n "$bucket" ]]
      ;;
    r2)
      [[ -n "$bucket" \
        && -n "$(release_storage_r2_endpoint)" \
        && -n "${RELEASE_R2_ACCESS_KEY_ID:-${CTX_RELEASE_R2_ACCESS_KEY_ID:-}}" \
        && -n "${RELEASE_R2_SECRET_ACCESS_KEY:-${CTX_RELEASE_R2_SECRET_ACCESS_KEY:-}}" ]]
      ;;
    *)
      return 1
      ;;
  esac
}

release_storage_require_write_env() {
  local provider bucket
  provider="$(release_storage_provider)"
  bucket="$(release_storage_bucket)"
  case "$provider" in
    supabase)
      if [[ -z "${SUPABASE_URL:-}" || -z "${SUPABASE_SERVICE_ROLE_KEY:-}" || -z "$bucket" ]]; then
        return 1
      fi
      ;;
    r2)
      if [[ -z "$bucket" ]]; then
        return 1
      fi
      if [[ -z "$(release_storage_r2_endpoint)" ]]; then
        return 1
      fi
      if [[ -z "${RELEASE_R2_ACCESS_KEY_ID:-${CTX_RELEASE_R2_ACCESS_KEY_ID:-}}" ]]; then
        return 1
      fi
      if [[ -z "${RELEASE_R2_SECRET_ACCESS_KEY:-${CTX_RELEASE_R2_SECRET_ACCESS_KEY:-}}" ]]; then
        return 1
      fi
      ;;
    *)
      return 1
      ;;
  esac
}

release_storage_export_normalized_env() {
  local provider bucket
  provider="$(release_storage_provider)"
  bucket="$(release_storage_bucket)"
  export RELEASE_STORAGE_PROVIDER="$provider"
  export RELEASE_STORAGE_BUCKET="$bucket"
  if [[ "$provider" == "supabase" && -z "${SUPABASE_STORAGE_BUCKET:-}" ]]; then
    export SUPABASE_STORAGE_BUCKET="$bucket"
  fi
}
