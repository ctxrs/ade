#!/usr/bin/env bash

release_storage_provider() {
  local provider="${RELEASE_STORAGE_PROVIDER:-${CTX_RELEASE_STORAGE_PROVIDER:-${CTX_RELEASE_STORAGE_BACKEND:-r2}}}"
  provider="$(printf '%s' "$provider" | tr 'A-Z' 'a-z')"
  if [[ "$provider" != "r2" ]]; then
    echo "error: RELEASE_STORAGE_PROVIDER must be r2 after the Cloudflare/R2 cutover (got '$provider')" >&2
    return 1
  fi
  printf '%s' "$provider"
}

release_storage_bucket() {
  printf '%s' "${RELEASE_STORAGE_BUCKET:-${CTX_RELEASES_R2_BUCKET:-${CTX_RELEASE_R2_BUCKET:-${RELEASE_R2_BUCKET:-}}}}"
}

release_storage_public_bucket() {
  printf '%s' "${RELEASE_PUBLIC_STORAGE_BUCKET:-releases}"
}

release_storage_public_origin() {
  local origin="${RELEASE_PUBLIC_STORAGE_ORIGIN:-https://api.ctx.rs}"
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
  local bucket
  release_storage_provider >/dev/null || return 1
  bucket="$(release_storage_bucket)"
  [[ -n "$bucket" \
    && -n "$(release_storage_r2_endpoint)" \
    && -n "${RELEASE_R2_ACCESS_KEY_ID:-${CTX_RELEASE_R2_ACCESS_KEY_ID:-}}" \
    && -n "${RELEASE_R2_SECRET_ACCESS_KEY:-${CTX_RELEASE_R2_SECRET_ACCESS_KEY:-}}" ]]
}

release_storage_require_write_env() {
  local bucket
  release_storage_provider >/dev/null || return 1
  bucket="$(release_storage_bucket)"
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
}

release_storage_export_normalized_env() {
  local bucket
  release_storage_provider >/dev/null || return 1
  bucket="$(release_storage_bucket)"
  export RELEASE_STORAGE_PROVIDER="r2"
  export RELEASE_STORAGE_BUCKET="$bucket"
}
