#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
source "$ROOT/scripts/lib/release_storage_env.sh"
export RELEASE_STORAGE_PROVIDER=""

if ! release_storage_has_write_env; then
  cat >&2 <<'EOF'
error: missing required R2 release storage env vars:
  - RELEASE_STORAGE_PROVIDER=r2
  - RELEASE_STORAGE_BUCKET or CTX_RELEASE_R2_BUCKET
  - RELEASE_R2_ENDPOINT or RELEASE_R2_ACCOUNT_ID
  - RELEASE_R2_ACCESS_KEY_ID
  - RELEASE_R2_SECRET_ACCESS_KEY

optional:
  - RELEASE_STORAGE_PROVIDER=r2
  - RELEASE_SOURCE_COMMIT (required; expected source commit embedded in versioned manifests)
  - RELEASE_CHANNEL (default: stable)
  - RELEASE_STORAGE_CHANNEL (default: RELEASE_CHANNEL; storage namespace override for dry runs)
  - RELEASE_VERSION (default: read from core/apps/desktop/package.json)
  - RELEASE_VERIFIED_MANIFEST_PATH (required; validated versioned manifest emitted by release_verify_storage.sh)
  - RELEASE_VERIFIED_MANIFEST_SIG_PATH (required; validated versioned manifest signature emitted by release_verify_storage.sh)
  - RELEASE_VERIFIED_TAURI_MANIFEST_PATH (required; validated versioned Tauri manifest emitted by release_verify_storage.sh)
EOF
  exit 2
fi
release_storage_export_normalized_env
if [[ -z "${RELEASE_SOURCE_COMMIT:-}" ]]; then
  echo "error: missing required env var: RELEASE_SOURCE_COMMIT" >&2
  exit 2
fi
VERIFIED_MANIFEST_PATH="$(printf '%s' "${RELEASE_VERIFIED_MANIFEST_PATH:-}" | xargs)"
VERIFIED_MANIFEST_SIG_PATH="$(printf '%s' "${RELEASE_VERIFIED_MANIFEST_SIG_PATH:-}" | xargs)"
VERIFIED_TAURI_MANIFEST_PATH="$(printf '%s' "${RELEASE_VERIFIED_TAURI_MANIFEST_PATH:-}" | xargs)"
if [[ -z "$VERIFIED_MANIFEST_PATH" || -z "$VERIFIED_MANIFEST_SIG_PATH" || -z "$VERIFIED_TAURI_MANIFEST_PATH" ]]; then
  echo "error: missing required env vars: RELEASE_VERIFIED_MANIFEST_PATH, RELEASE_VERIFIED_MANIFEST_SIG_PATH, and RELEASE_VERIFIED_TAURI_MANIFEST_PATH" >&2
  exit 2
fi
if [[ ! -f "$VERIFIED_MANIFEST_PATH" ]]; then
  echo "error: verified version manifest file does not exist: $VERIFIED_MANIFEST_PATH" >&2
  exit 2
fi
if [[ ! -s "$VERIFIED_MANIFEST_SIG_PATH" ]]; then
  echo "error: verified version manifest signature file does not exist or is empty: $VERIFIED_MANIFEST_SIG_PATH" >&2
  exit 2
fi
if [[ ! -f "$VERIFIED_TAURI_MANIFEST_PATH" ]]; then
  echo "error: verified version tauri manifest file does not exist: $VERIFIED_TAURI_MANIFEST_PATH" >&2
  exit 2
fi

CHANNEL="${RELEASE_CHANNEL:-stable}"
STORAGE_CHANNEL="${RELEASE_STORAGE_CHANNEL:-$CHANNEL}"
if ! [[ "$STORAGE_CHANNEL" =~ ^[A-Za-z0-9._-]+$ ]]; then
  echo "error: invalid RELEASE_STORAGE_CHANNEL '$STORAGE_CHANNEL' (allowed chars: A-Z a-z 0-9 . _ -)" >&2
  exit 2
fi
PROMOTE_CURL_CONNECT_TIMEOUT_SECONDS="${RELEASE_PROMOTE_CURL_CONNECT_TIMEOUT_SECONDS:-20}"
PROMOTE_CURL_MAX_TIME_SECONDS="${RELEASE_PROMOTE_CURL_MAX_TIME_SECONDS:-300}"
PROMOTE_CURL_STALL_SECONDS="${RELEASE_PROMOTE_CURL_STALL_SECONDS:-60}"
PROMOTE_CURL_STALL_BYTES_PER_SECOND="${RELEASE_PROMOTE_CURL_STALL_BYTES_PER_SECOND:-1024}"
for numeric_var in \
  PROMOTE_CURL_CONNECT_TIMEOUT_SECONDS \
  PROMOTE_CURL_MAX_TIME_SECONDS \
  PROMOTE_CURL_STALL_SECONDS \
  PROMOTE_CURL_STALL_BYTES_PER_SECOND; do
  if ! [[ "${!numeric_var}" =~ ^[0-9]+$ ]] || (( "${!numeric_var}" < 1 )); then
    echo "error: invalid ${numeric_var}=${!numeric_var} (expected positive integer)" >&2
    exit 2
  fi
done
node ./core/scripts/desktop_check_versions.cjs >/dev/null
if [[ -n "${RELEASE_VERSION:-}" ]]; then
  VERSION="$RELEASE_VERSION"
else
  VERSION="$(node -p "require('./core/apps/desktop/package.json').version")"
fi

STORAGE_PROVIDER="$(release_storage_provider)"
API="${RELEASE_STORAGE_API_ORIGIN:-}"
API="${API%/}"
PUBLIC_API="$(release_storage_public_origin)"
BUCKET="$(release_storage_bucket)"
PUBLIC_BUCKET="$(release_storage_public_bucket)"
AUTH="authorization: Bearer ${RELEASE_STORAGE_SERVICE_TOKEN:-}"
APIKEY="apikey: ${RELEASE_STORAGE_SERVICE_TOKEN:-}"
STAGING="$(mktemp -d /tmp/ctx-release-promote.XXXXXX)"
MANIFEST_LOCK_PATH=""

declare -a PROMOTE_UPLOAD_CURL_ARGS=(
  --connect-timeout "$PROMOTE_CURL_CONNECT_TIMEOUT_SECONDS"
  --max-time "$PROMOTE_CURL_MAX_TIME_SECONDS"
  --speed-time "$PROMOTE_CURL_STALL_SECONDS"
  --speed-limit "$PROMOTE_CURL_STALL_BYTES_PER_SECOND"
)

declare -a PROMOTE_CONTROL_CURL_ARGS=(
  --connect-timeout "$PROMOTE_CURL_CONNECT_TIMEOUT_SECONDS"
  --max-time "$PROMOTE_CURL_MAX_TIME_SECONDS"
)

release_manifest_lock() {
  if [[ -z "${MANIFEST_LOCK_PATH:-}" ]]; then
    return 0
  fi
  if [[ "$STORAGE_PROVIDER" == "r2" ]]; then
    storage_cli delete --object-path "$MANIFEST_LOCK_PATH" >/dev/null || true
    MANIFEST_LOCK_PATH=""
    return 0
  fi
  curl -fsS "${PROMOTE_CONTROL_CURL_ARGS[@]}" -X DELETE "$API/storage/v1/object/$BUCKET/$MANIFEST_LOCK_PATH" \
    -H "$AUTH" \
    -H "$APIKEY" >/dev/null || true
  MANIFEST_LOCK_PATH=""
}
trap 'release_manifest_lock; rm -rf "$STAGING"' EXIT

storage_cli() {
  RELEASE_STORAGE_PROVIDER="$STORAGE_PROVIDER" RELEASE_STORAGE_BUCKET="$BUCKET" \
    node ./core/scripts/release_storage_object.cjs "$@"
}

upload_object() {
  local src="$1"
  local object_path="$2"
  local content_type="$3"
  if [[ "$STORAGE_PROVIDER" == "r2" ]]; then
    for attempt in 1 2 3; do
      echo "Uploading object '$object_path' (attempt ${attempt}/3)..."
      if storage_cli put --src "$src" --object-path "$object_path" --content-type "$content_type" --upsert true; then
        return 0
      fi
      if (( attempt < 3 )); then
        echo "warn: upload failed for '$object_path' attempt $attempt/3; retrying..." >&2
        sleep $((attempt * 5))
      fi
    done
    echo "error: failed to upload object '$object_path'" >&2
    return 1
  fi
  local response_file="$STAGING/upload-response.json"
  local status
  for attempt in 1 2 3; do
    echo "Uploading object '$object_path' (attempt ${attempt}/3)..."
    status="$(curl -sS "${PROMOTE_UPLOAD_CURL_ARGS[@]}" -o "$response_file" -w '%{http_code}' -X POST "$API/storage/v1/object/$BUCKET/$object_path" \
      -H "$AUTH" \
      -H "$APIKEY" \
      -H "x-upsert: true" \
      -H "content-type: $content_type" \
      --upload-file "$src" || true)"
    case "$status" in
      200|201|204)
        return 0
        ;;
    esac
    if (( attempt < 3 )); then
      echo "warn: upload failed for '$object_path' (status=${status:-unknown}) attempt $attempt/3; retrying..." >&2
      if [[ -s "$response_file" ]]; then
        cat "$response_file" >&2
      fi
      sleep $((attempt * 5))
    fi
  done
  echo "error: failed to upload object '$object_path' (status=${status:-unknown})" >&2
  if [[ -s "$response_file" ]]; then
    cat "$response_file" >&2
  fi
  return 1
}

acquire_manifest_lock() {
  local lock_path="releases/$STORAGE_CHANNEL/latest.manifest.lock"
  local lock_payload="$STAGING/manifest-lock.json"
  local wait_seconds="${RELEASE_MANIFEST_LOCK_WAIT_SECONDS:-180}"
  local sleep_seconds=2
  local max_attempts
  max_attempts=$((wait_seconds / sleep_seconds))
  if (( max_attempts < 1 )); then
    max_attempts=1
  fi

  printf '{"channel":"%s","version":"%s","action":"promote-latest","created_at":"%s","pid":%s}\n' \
    "$CHANNEL" "$VERSION" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$$" >"$lock_payload"

  local attempt=1
  while (( attempt <= max_attempts )); do
    if [[ "$STORAGE_PROVIDER" == "r2" ]]; then
      if storage_cli put --src "$lock_payload" --object-path "$lock_path" --content-type "application/json" --upsert false >/dev/null; then
        MANIFEST_LOCK_PATH="$lock_path"
        echo "Acquired manifest lock: $lock_path"
        return 0
      fi
      if (( attempt == max_attempts )); then
        echo "error: failed to acquire manifest lock after ${wait_seconds}s: $lock_path" >&2
        echo "hint: if a prior publish run was interrupted, delete the stale lock object and retry." >&2
        return 1
      fi
      sleep "$sleep_seconds"
      attempt=$((attempt + 1))
      continue
    fi
    if curl -fsS "${PROMOTE_CONTROL_CURL_ARGS[@]}" -X POST "$API/storage/v1/object/$BUCKET/$lock_path" \
      -H "$AUTH" \
      -H "$APIKEY" \
      -H "x-upsert: false" \
      -H "content-type: application/json" \
      --data-binary @"$lock_payload" >/dev/null; then
      MANIFEST_LOCK_PATH="$lock_path"
      echo "Acquired manifest lock: $lock_path"
      return 0
    fi
    if (( attempt == max_attempts )); then
      echo "error: failed to acquire manifest lock after ${wait_seconds}s: $lock_path" >&2
      echo "hint: if a prior publish run was interrupted, delete the stale lock object and retry." >&2
      return 1
    fi
    sleep "$sleep_seconds"
    attempt=$((attempt + 1))
  done
}

validate_versioned_manifests() {
  EXPECTED_VERSION="$VERSION" \
  EXPECTED_SOURCE_COMMIT="$RELEASE_SOURCE_COMMIT" \
  EXPECTED_CHANNEL="$CHANNEL" \
  VERSION_MANIFEST_PATH="$version_manifest_file" \
  VERSION_TAURI_MANIFEST_PATH="$version_tauri_manifest_file" \
  node - <<'NODE'
const fs = require("fs");

function fail(msg) {
  console.error(`error: ${msg}`);
  process.exit(1);
}

const expectedVersion = String(process.env.EXPECTED_VERSION || "").trim();
const expectedSourceCommit = String(process.env.EXPECTED_SOURCE_COMMIT || "").trim();
const expectedChannel = String(process.env.EXPECTED_CHANNEL || "").trim();
if (!expectedVersion) fail("EXPECTED_VERSION missing");
if (!expectedSourceCommit) fail("EXPECTED_SOURCE_COMMIT missing");
if (!expectedChannel) fail("EXPECTED_CHANNEL missing");

const latest = JSON.parse(fs.readFileSync(process.env.VERSION_MANIFEST_PATH, "utf8"));
const tauri = JSON.parse(fs.readFileSync(process.env.VERSION_TAURI_MANIFEST_PATH, "utf8"));

if (String(latest.channel || "") !== expectedChannel) {
  fail(`version manifest channel mismatch: expected ${expectedChannel}, got ${latest.channel || "<missing>"}`);
}
if (String(latest.latest_version || "") !== expectedVersion) {
  fail(`version manifest mismatch: expected ${expectedVersion}, got ${latest.latest_version || "<missing>"}`);
}
if (String(tauri.version || "") !== expectedVersion) {
  fail(`tauri manifest mismatch: expected ${expectedVersion}, got ${tauri.version || "<missing>"}`);
}
if (String(latest.source_commit || "") !== expectedSourceCommit) {
  fail(`version manifest source_commit mismatch: expected ${expectedSourceCommit}, got ${latest.source_commit || "<missing>"}`);
}
if (String(tauri.source_commit || "") !== expectedSourceCommit) {
  fail(`tauri manifest source_commit mismatch: expected ${expectedSourceCommit}, got ${tauri.source_commit || "<missing>"}`);
}
if (!latest.platforms || typeof latest.platforms !== "object" || Object.keys(latest.platforms).length === 0) {
  fail("version manifest has no platforms");
}
if (!tauri.platforms || typeof tauri.platforms !== "object" || Object.keys(tauri.platforms).length === 0) {
  fail("tauri manifest has no platforms");
}
NODE
}

stage_verified_manifests() {
  cp -f "$VERIFIED_MANIFEST_PATH" "$version_manifest_file"
  cp -f "$VERIFIED_MANIFEST_SIG_PATH" "$version_manifest_sig_file"
  cp -f "$VERIFIED_TAURI_MANIFEST_PATH" "$version_tauri_manifest_file"
  node core/scripts/verify_release_manifest_signature.cjs "$version_manifest_file" "$version_manifest_sig_file"
  validate_versioned_manifests
}

latest_manifest_path="releases/$STORAGE_CHANNEL/latest.json"
latest_manifest_sig_path="releases/$STORAGE_CHANNEL/latest.json.sig"
latest_tauri_manifest_path="releases/$STORAGE_CHANNEL/latest-tauri.json"
version_manifest_file="$STAGING/${VERSION}.json"
version_manifest_sig_file="$STAGING/${VERSION}.json.sig"
version_tauri_manifest_file="$STAGING/${VERSION}-tauri.json"

stage_verified_manifests

echo "Promoting versioned manifests to latest aliases..."
acquire_manifest_lock
upload_object "$version_manifest_file" "$latest_manifest_path" "application/json"
upload_object "$version_manifest_sig_file" "$latest_manifest_sig_path" "text/plain"
upload_object "$version_tauri_manifest_file" "$latest_tauri_manifest_path" "application/json"
release_manifest_lock

echo "Promoted release manifests for $CHANNEL@$VERSION (storage namespace $STORAGE_CHANNEL):"
echo "- $PUBLIC_API/storage/v1/object/public/$PUBLIC_BUCKET/$latest_manifest_path"
echo "- $PUBLIC_API/storage/v1/object/public/$PUBLIC_BUCKET/$latest_manifest_sig_path"
echo "- $PUBLIC_API/storage/v1/object/public/$PUBLIC_BUCKET/$latest_tauri_manifest_path"
