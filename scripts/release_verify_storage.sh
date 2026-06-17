#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ -z "${RELEASE_FUNCTIONS_URL:-}" && -z "${RELEASE_PUBLIC_STORAGE_ORIGIN:-}" && -z "${RELEASE_FUNCTIONS_URL:-}" && -z "${RELEASE_PUBLIC_STORAGE_ORIGIN:-}" ]]; then
  cat >&2 <<'EOF'
error: missing base URL for release verification

Examples:
  - RELEASE_FUNCTIONS_URL=https://api.ctx.rs/functions/v1
  - RELEASE_PUBLIC_STORAGE_ORIGIN=https://api.ctx.rs (fallback derives /functions/v1)
  - RELEASE_FUNCTIONS_URL=http://127.0.0.1:54321/functions/v1
  - RELEASE_FUNCTIONS_URL=https://api.ctx.rs/functions/v1
  - RELEASE_PUBLIC_STORAGE_ORIGIN=https://api.ctx.rs (fallback derives /functions/v1)

optional:
  - RELEASE_SOURCE_COMMIT=<sha> (required; expected source commit embedded in release manifests)
  - RELEASE_VERIFY_VERSION=<version> (verify /releases/<channel>/<version>.json and <version>-tauri.json instead of latest aliases)
  - RELEASE_CHANNEL (default: stable; allowed: stable, canary, preview, e2e)
  - RELEASE_STORAGE_CHANNEL=<path-segment> (default: RELEASE_CHANNEL; storage namespace override for dry runs)
  - RELEASE_VERIFIED_MANIFEST_PATH=<path> (optional; when RELEASE_VERIFY_VERSION is set, write the validated versioned manifest here)
  - RELEASE_VERIFIED_MANIFEST_SIG_PATH=<path> (optional; required with RELEASE_VERIFIED_MANIFEST_PATH, writes validated <version>.json.sig)
  - RELEASE_VERIFIED_TAURI_MANIFEST_PATH=<path> (optional; when RELEASE_VERIFY_VERSION is set, write the validated versioned Tauri manifest here)
  - RELEASE_VERIFY_ARTIFACT_CURL_MAX_TIME_SECONDS=<seconds> (optional; updater artifact fetch budget, default: 1800)
  - RELEASE_VERIFY_FINAL_ARTIFACT_ANALYTICS=0|1 (optional; verify extracted Linux AppImage analytics, default: 1)
EOF
  exit 2
fi
if [[ -z "${RELEASE_SOURCE_COMMIT:-}" ]]; then
  echo "error: missing required env var: RELEASE_SOURCE_COMMIT" >&2
  exit 2
fi

CHANNEL="${RELEASE_CHANNEL:-stable}"
case "$CHANNEL" in
  stable|canary|preview|e2e) ;;
  *)
    echo "error: unsupported RELEASE_CHANNEL '$CHANNEL' (allowed: stable, canary, preview, e2e)" >&2
    exit 2
    ;;
esac
STORAGE_CHANNEL="${RELEASE_STORAGE_CHANNEL:-$CHANNEL}"
if ! [[ "$STORAGE_CHANNEL" =~ ^[A-Za-z0-9._-]+$ ]]; then
  echo "error: invalid RELEASE_STORAGE_CHANNEL '$STORAGE_CHANNEL' (allowed chars: A-Z a-z 0-9 . _ -)" >&2
  exit 2
fi
VERIFY_VERSION="$(printf '%s' "${RELEASE_VERIFY_VERSION:-}" | xargs)"
EXPECTED_PLATFORMS="${RELEASE_EXPECTED_PLATFORMS:-}"
VERIFY_SCOPE="${RELEASE_VERIFY_SCOPE:-all}"
VERIFY_RETRIES="${RELEASE_VERIFY_RETRIES:-8}"
VERIFY_RETRY_SLEEP_SECONDS="${RELEASE_VERIFY_RETRY_SLEEP_SECONDS:-3}"
VERIFY_BASE_DISCOVERY_RETRIES="${RELEASE_VERIFY_BASE_DISCOVERY_RETRIES:-2}"
VERIFY_VALIDATION_RETRIES="${RELEASE_VERIFY_VALIDATION_RETRIES:-2}"
VERIFY_PROBE_PLATFORM="${RELEASE_VERIFY_PROBE_PLATFORM:-${EXPECTED_PLATFORMS%%,*}}"
VERIFY_CACHE_BUST_SEED="${RELEASE_VERIFY_CACHE_BUST_SEED:-$(date -u +%s%N)}"
VERIFY_ALLOW_HTTP_UPDATER_URLS="${RELEASE_VERIFY_ALLOW_HTTP_UPDATER_URLS:-0}"
VERIFY_CURL_CONNECT_TIMEOUT_SECONDS="${RELEASE_VERIFY_CURL_CONNECT_TIMEOUT_SECONDS:-20}"
VERIFY_CURL_MAX_TIME_SECONDS="${RELEASE_VERIFY_CURL_MAX_TIME_SECONDS:-600}"
VERIFY_ARTIFACT_CURL_MAX_TIME_SECONDS="${RELEASE_VERIFY_ARTIFACT_CURL_MAX_TIME_SECONDS:-1800}"
VERIFY_ARTIFACT_FETCH_RETRIES="${RELEASE_VERIFY_ARTIFACT_FETCH_RETRIES:-1}"
VERIFY_LINUX_REMOTE_ARTIFACTS="${RELEASE_VERIFY_LINUX_REMOTE_ARTIFACTS:-auto}"
VERIFY_LINUX_REMOTE_ARTIFACT_EMULATION="${RELEASE_VERIFY_LINUX_REMOTE_ARTIFACT_EMULATION:-0}"
VERIFY_FINAL_ARTIFACT_ANALYTICS="${RELEASE_VERIFY_FINAL_ARTIFACT_ANALYTICS:-1}"
VERIFY_CURL_STALL_SECONDS="${RELEASE_VERIFY_CURL_STALL_SECONDS:-60}"
VERIFY_CURL_STALL_BYTES_PER_SECOND="${RELEASE_VERIFY_CURL_STALL_BYTES_PER_SECOND:-1024}"
VERIFY_MIN_DMG_SIZE_BYTES="${RELEASE_MIN_DMG_SIZE_BYTES:-1048576}"
VERIFIED_MANIFEST_PATH="$(printf '%s' "${RELEASE_VERIFIED_MANIFEST_PATH:-}" | xargs)"
VERIFIED_MANIFEST_SIG_PATH="$(printf '%s' "${RELEASE_VERIFIED_MANIFEST_SIG_PATH:-}" | xargs)"
VERIFIED_TAURI_MANIFEST_PATH="$(printf '%s' "${RELEASE_VERIFIED_TAURI_MANIFEST_PATH:-}" | xargs)"
VERIFY_PROBE_PLATFORM="$(printf '%s' "${VERIFY_PROBE_PLATFORM:-}" | xargs)"
if [[ -z "$VERIFY_PROBE_PLATFORM" ]]; then
  VERIFY_PROBE_PLATFORM="linux-x64"
fi
for numeric_var in \
  VERIFY_VALIDATION_RETRIES \
  VERIFY_CURL_CONNECT_TIMEOUT_SECONDS \
  VERIFY_CURL_MAX_TIME_SECONDS \
  VERIFY_ARTIFACT_CURL_MAX_TIME_SECONDS \
  VERIFY_ARTIFACT_FETCH_RETRIES \
  VERIFY_CURL_STALL_SECONDS \
  VERIFY_CURL_STALL_BYTES_PER_SECOND \
  VERIFY_MIN_DMG_SIZE_BYTES; do
  if ! [[ "${!numeric_var}" =~ ^[0-9]+$ ]] || (( "${!numeric_var}" < 1 )); then
    echo "error: invalid ${numeric_var}=${!numeric_var} (expected positive integer)" >&2
    exit 2
  fi
done
case "$VERIFY_SCOPE" in
  all|expected) ;;
  *)
    echo "error: invalid RELEASE_VERIFY_SCOPE=$VERIFY_SCOPE (expected: all|expected)" >&2
    exit 2
    ;;
esac
if [[ -n "$VERIFIED_MANIFEST_PATH" || -n "$VERIFIED_MANIFEST_SIG_PATH" || -n "$VERIFIED_TAURI_MANIFEST_PATH" ]]; then
  if [[ -z "$VERIFIED_MANIFEST_PATH" || -z "$VERIFIED_MANIFEST_SIG_PATH" || -z "$VERIFIED_TAURI_MANIFEST_PATH" ]]; then
    echo "error: RELEASE_VERIFIED_MANIFEST_PATH, RELEASE_VERIFIED_MANIFEST_SIG_PATH, and RELEASE_VERIFIED_TAURI_MANIFEST_PATH must be set together" >&2
    exit 2
  fi
  if [[ -z "$VERIFY_VERSION" ]]; then
    echo "error: RELEASE_VERIFY_VERSION is required when writing verified release manifests" >&2
    exit 2
  fi
fi

declare -a VERIFY_FETCH_CURL_ARGS=(
  --connect-timeout "$VERIFY_CURL_CONNECT_TIMEOUT_SECONDS"
  --max-time "$VERIFY_CURL_MAX_TIME_SECONDS"
  --speed-time "$VERIFY_CURL_STALL_SECONDS"
  --speed-limit "$VERIFY_CURL_STALL_BYTES_PER_SECOND"
)

declare -a VERIFY_ARTIFACT_FETCH_CURL_ARGS=(
  --connect-timeout "$VERIFY_CURL_CONNECT_TIMEOUT_SECONDS"
  --max-time "$VERIFY_ARTIFACT_CURL_MAX_TIME_SECONDS"
  --speed-time "$VERIFY_CURL_STALL_SECONDS"
  --speed-limit "$VERIFY_CURL_STALL_BYTES_PER_SECOND"
)

declare -a VERIFY_HEAD_CURL_ARGS=(
  --connect-timeout "$VERIFY_CURL_CONNECT_TIMEOUT_SECONDS"
  --max-time "$VERIFY_CURL_MAX_TIME_SECONDS"
)

fetch_with_retry() {
  local url="$1"
  local out="$2"
  local label="$3"
  local attempts="${4:-$VERIFY_RETRIES}"
  local attempt=1
  while (( attempt <= attempts )); do
    echo "Fetching $label (attempt ${attempt}/${attempts}): $url"
    if curl -fsSL "${VERIFY_FETCH_CURL_ARGS[@]}" "$url" -o "$out"; then
      return 0
    fi
    if (( attempt == attempts )); then
      echo "error: failed to fetch $label after ${attempts} attempts: $url" >&2
      return 1
    fi
    sleep "$VERIFY_RETRY_SLEEP_SECONDS"
    attempt=$((attempt + 1))
  done
}

fetch_artifact_with_retry() {
  local url="$1"
  local out="$2"
  local label="$3"
  local attempts="${4:-$VERIFY_ARTIFACT_FETCH_RETRIES}"
  local attempt=1
  while (( attempt <= attempts )); do
    echo "Fetching $label (attempt ${attempt}/${attempts}): $url"
    if curl -fsSL "${VERIFY_ARTIFACT_FETCH_CURL_ARGS[@]}" "$url" -o "$out"; then
      return 0
    fi
    if (( attempt == attempts )); then
      echo "error: failed to fetch $label after ${attempts} attempts: $url" >&2
      return 1
    fi
    sleep "$VERIFY_RETRY_SLEEP_SECONDS"
    attempt=$((attempt + 1))
  done
}

verify_file_size_bytes() {
  local path="$1"
  if stat -c '%s' "$path" >/dev/null 2>&1; then
    stat -c '%s' "$path"
  else
    stat -f '%z' "$path"
  fi
}

sha256_file() {
  local path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{print $1}'
    return
  fi
  shasum -a 256 "$path" | awk '{print $1}'
}

verify_sha256() {
  local path="$1"
  local expected="$2"
  local label="$3"
  local actual
  local actual_lower expected_lower
  actual="$(sha256_file "$path")"
  actual_lower="$(printf '%s' "$actual" | tr 'A-F' 'a-f')"
  expected_lower="$(printf '%s' "$expected" | tr 'A-F' 'a-f')"
  if [[ "$actual_lower" != "$expected_lower" ]]; then
    echo "error: ${label} sha256 mismatch (expected ${expected}, got ${actual})" >&2
    return 1
  fi
}

host_linux_release_platform() {
  if [[ "$(uname -s)" != "Linux" ]]; then
    return 1
  fi
  case "$(uname -m)" in
    x86_64|amd64) printf 'linux-x64\n' ;;
    aarch64|arm64) printf 'linux-arm64\n' ;;
    *) return 1 ;;
  esac
}

should_verify_linux_remote_artifact_platform() {
  local platform="$1"
  local mode="$VERIFY_LINUX_REMOTE_ARTIFACTS"
  local host_platform=""
  case "$mode" in
    0|false|FALSE|no|NO|off|OFF) return 1 ;;
  esac
  host_platform="$(host_linux_release_platform || true)"
  if [[ -z "$host_platform" ]]; then
    [[ "$mode" == "1" || "$mode" == "true" || "$mode" == "TRUE" ]] && {
      echo "error: RELEASE_VERIFY_LINUX_REMOTE_ARTIFACTS=${mode} requires a supported Linux host" >&2
      return 2
    }
    return 1
  fi
  if [[ "$platform" == "$host_platform" ]]; then
    return 0
  fi
  if [[ "$mode" == "all" && "$VERIFY_LINUX_REMOTE_ARTIFACT_EMULATION" == "1" ]]; then
    return 0
  fi
  return 1
}

linux_remote_artifact_verification_disabled() {
  case "$VERIFY_LINUX_REMOTE_ARTIFACTS" in
    0|false|FALSE|no|NO|off|OFF) return 0 ;;
  esac
  return 1
}

validate_linux_daemon_artifact() {
  local url="$1"
  local sha="$2"
  local platform="$3"
  local artifact_path="$tmp_updater_dir/${platform}-daemon"
  local help_log="$tmp_updater_dir/${platform}-daemon-help.log"
  local should_status=0
  if linux_remote_artifact_verification_disabled; then
    echo "- linux daemon execution skipped: ${platform} (disabled)"
    return 0
  fi
  fetch_artifact_with_retry "$url" "$artifact_path" "${platform}/daemon artifact" || return 1
  verify_sha256 "$artifact_path" "$sha" "${platform}/daemon" || return 1
  should_verify_linux_remote_artifact_platform "$platform" || should_status=$?
  if [[ "$should_status" == "2" ]]; then
    return 1
  fi
  if [[ "$should_status" != "0" ]]; then
    echo "- linux daemon execution skipped: ${platform} (host architecture mismatch or disabled)"
    return 0
  fi
  chmod +x "$artifact_path"
  if ! "$artifact_path" serve --help >"$help_log" 2>&1; then
    echo "error: ${platform}/daemon failed serve --help" >&2
    cat "$help_log" >&2 || true
    return 1
  fi
  echo "- linux daemon verify OK: ${platform}"
}

validate_linux_appimage_artifact() {
  local url="$1"
  local sha="$2"
  local platform="$3"
  local artifact_path="$tmp_updater_dir/${platform}.AppImage"
  local extract_parent="$tmp_updater_dir/${platform}-appimage-extract"
  local extract_log="$tmp_updater_dir/${platform}-appimage-extract.log"
  local should_status=0
  if linux_remote_artifact_verification_disabled; then
    echo "- linux AppImage extraction skipped: ${platform} (disabled)"
    return 0
  fi
  fetch_artifact_with_retry "$url" "$artifact_path" "${platform}/AppImage artifact" || return 1
  verify_sha256 "$artifact_path" "$sha" "${platform}/AppImage" || return 1
  should_verify_linux_remote_artifact_platform "$platform" || should_status=$?
  if [[ "$should_status" == "2" ]]; then
    return 1
  fi
  if [[ "$should_status" != "0" ]]; then
    echo "- linux AppImage extraction skipped: ${platform} (host architecture mismatch or disabled)"
    return 0
  fi
  chmod +x "$artifact_path"
  rm -rf "$extract_parent"
  mkdir -p "$extract_parent"
  if ! (cd "$extract_parent" && "$artifact_path" --appimage-extract >"$extract_log" 2>&1); then
    echo "error: ${platform}/AppImage failed --appimage-extract" >&2
    cat "$extract_log" >&2 || true
    return 1
  fi
  local squashfs_root="$extract_parent/squashfs-root"
  if [[ ! -d "$squashfs_root" ]]; then
    echo "error: ${platform}/AppImage extraction did not create squashfs-root" >&2
    return 1
  fi
  local bundle_manifest=""
  bundle_manifest="$(find "$squashfs_root" -path '*/bundles/manifest.json' -type f -print -quit)"
  if [[ -z "$bundle_manifest" ]]; then
    echo "error: ${platform}/AppImage missing bundles/manifest.json required by remote bootstrap" >&2
    return 1
  fi
  if ! node core/scripts/verify_bundle_manifest_closure.cjs "$(dirname "$bundle_manifest")"; then
    echo "error: ${platform}/AppImage bundle manifest references files that were not packaged" >&2
    return 1
  fi
  if [[ "$VERIFY_FINAL_ARTIFACT_ANALYTICS" == "1" ]]; then
    if [[ -z "${verified_release_version:-}" ]]; then
      echo "error: cannot verify ${platform}/AppImage analytics without a resolved release version" >&2
      return 1
    fi
    if ! node core/scripts/final_artifact_analytics_gate.cjs \
      --artifact-root "$squashfs_root" \
      --expected-version "$verified_release_version"; then
      echo "error: ${platform}/AppImage final artifact analytics gate failed" >&2
      return 1
    fi
  fi
  if ! find "$squashfs_root" \( -name 'ctx' -o -name 'ctx-daemon-*' \) -type f -print -quit | grep -q .; then
    echo "error: ${platform}/AppImage missing bundled ctx daemon binary" >&2
    return 1
  fi
  echo "- linux AppImage layout verify OK: ${platform}"
}

validate_remote_macos_dmg_artifact() {
  local url="$1"
  local label="$2"
  local safe_label="${label//[^A-Za-z0-9._-]/_}"
  local headers_path="$tmp_updater_dir/${safe_label}.headers"
  local probe_path="$tmp_updater_dir/${safe_label}.dmg.probe"
  local content_length=""
  local magic=""

  if ! curl -fsSLI "${VERIFY_HEAD_CURL_ARGS[@]}" "$url" -o "$headers_path"; then
    echo "error: failed to fetch headers for $label at $url" >&2
    return 1
  fi
  content_length="$(awk 'BEGIN {IGNORECASE=1} /^content-length:/ {gsub("\r", "", $2); value=$2} END {print value}' "$headers_path")"
  if [[ "$content_length" =~ ^[0-9]+$ ]] && (( content_length < VERIFY_MIN_DMG_SIZE_BYTES )); then
    echo "error: $label is suspiciously small (${content_length} bytes, minimum ${VERIFY_MIN_DMG_SIZE_BYTES})" >&2
    return 1
  fi
  if [[ ! "$content_length" =~ ^[0-9]+$ ]]; then
    local range_end=$((VERIFY_MIN_DMG_SIZE_BYTES - 1))
    local probed_size=""
    if ! curl -fsSL "${VERIFY_ARTIFACT_FETCH_CURL_ARGS[@]}" --range "0-${range_end}" "$url" -o "$probe_path"; then
      echo "error: failed to fetch DMG size probe for $label at $url" >&2
      return 1
    fi
    probed_size="$(verify_file_size_bytes "$probe_path")"
    if [[ ! "$probed_size" =~ ^[0-9]+$ ]] || (( probed_size < VERIFY_MIN_DMG_SIZE_BYTES )); then
      echo "error: $label size could not be proven (${probed_size:-unknown} bytes fetched, minimum ${VERIFY_MIN_DMG_SIZE_BYTES})" >&2
      return 1
    fi
    magic="$(od -An -tx1 -N4 "$probe_path" | tr -d '[:space:]')"
    if [[ "$magic" == "00051607" ]]; then
      echo "error: $label is an AppleDouble sidecar, not a DMG artifact" >&2
      return 1
    fi
    return 0
  fi

  if ! curl -fsSL "${VERIFY_ARTIFACT_FETCH_CURL_ARGS[@]}" --range 0-3 "$url" -o "$probe_path"; then
    echo "error: failed to fetch DMG header probe for $label at $url" >&2
    return 1
  fi
  magic="$(od -An -tx1 -N4 "$probe_path" | tr -d '[:space:]')"
  if [[ "$magic" == "00051607" ]]; then
    echo "error: $label is an AppleDouble sidecar, not a DMG artifact" >&2
    return 1
  fi
}

head_redirect_code_with_retry() {
  local url="$1"
  local label="$2"
  local attempt=1
  local code
  while (( attempt <= VERIFY_RETRIES )); do
    code="$(curl -sS "${VERIFY_HEAD_CURL_ARGS[@]}" -o /dev/null -I -w '%{http_code}' "$url" || true)"
    if [[ "$code" == "301" || "$code" == "302" ]]; then
      printf '%s' "$code"
      return 0
    fi
    if (( attempt == VERIFY_RETRIES )); then
      echo "error: expected redirect for $label at $url, got $code after ${VERIFY_RETRIES} attempts" >&2
      return 1
    fi
    sleep "$VERIFY_RETRY_SLEEP_SECONDS"
    attempt=$((attempt + 1))
  done
}

tmp="$(mktemp "${TMPDIR:-/tmp}/ctx-latest.XXXXXX")"
tmp_sig="$(mktemp "${TMPDIR:-/tmp}/ctx-latest-sig.XXXXXX")"
tmp_tauri="$(mktemp "${TMPDIR:-/tmp}/ctx-latest-tauri.XXXXXX")"
tmp_updater_dir="$(mktemp -d /tmp/ctx-updater-verify.XXXXXX)"
trap 'rm -f "$tmp" "$tmp_sig" "$tmp_tauri"; rm -rf "$tmp_updater_dir"' EXIT

RELEASE_FUNCTIONS_URL_CLEAN="${RELEASE_FUNCTIONS_URL:-${RELEASE_FUNCTIONS_URL:-}}"
RELEASE_FUNCTIONS_URL_CLEAN="${RELEASE_FUNCTIONS_URL_CLEAN%/}"
RELEASE_PUBLIC_STORAGE_ORIGIN_CLEAN="${RELEASE_PUBLIC_STORAGE_ORIGIN:-${RELEASE_PUBLIC_STORAGE_ORIGIN:-}}"
RELEASE_PUBLIC_STORAGE_ORIGIN_CLEAN="${RELEASE_PUBLIC_STORAGE_ORIGIN_CLEAN%/}"

declare -a CANDIDATE_BASES=()
if [[ -n "$RELEASE_FUNCTIONS_URL_CLEAN" ]]; then
  CANDIDATE_BASES+=("$RELEASE_FUNCTIONS_URL_CLEAN")
fi
if [[ -n "$RELEASE_PUBLIC_STORAGE_ORIGIN_CLEAN" ]]; then
  derived="${RELEASE_PUBLIC_STORAGE_ORIGIN_CLEAN}/functions/v1"
  duplicate=0
  for base in "${CANDIDATE_BASES[@]:-}"; do
    if [[ "$base" == "$derived" ]]; then
      duplicate=1
      break
    fi
  done
  if (( duplicate == 0 )); then
    CANDIDATE_BASES+=("$derived")
  fi
fi

BASE=""
for candidate in "${CANDIDATE_BASES[@]}"; do
  candidate_no_slash="${candidate%/}"
  manifest_base="$candidate_no_slash"
  download_base="$candidate_no_slash"
  if [[ "$candidate_no_slash" == */releases ]]; then
    if [[ -n "$VERIFY_VERSION" ]]; then
      latest_url="${manifest_base}/$STORAGE_CHANNEL/${VERIFY_VERSION}.json"
      latest_sig_url="${manifest_base}/$STORAGE_CHANNEL/${VERIFY_VERSION}.json.sig"
      latest_tauri_url="${manifest_base}/$STORAGE_CHANNEL/${VERIFY_VERSION}-tauri.json"
    else
      latest_url="${manifest_base}/$STORAGE_CHANNEL/latest.json"
      latest_sig_url="${manifest_base}/$STORAGE_CHANNEL/latest.json.sig"
      latest_tauri_url="${manifest_base}/$STORAGE_CHANNEL/latest-tauri.json"
    fi
    download_base="${candidate_no_slash%/releases}"
  else
    if [[ -n "$VERIFY_VERSION" ]]; then
      latest_url="${manifest_base}/releases/$STORAGE_CHANNEL/${VERIFY_VERSION}.json"
      latest_sig_url="${manifest_base}/releases/$STORAGE_CHANNEL/${VERIFY_VERSION}.json.sig"
      latest_tauri_url="${manifest_base}/releases/$STORAGE_CHANNEL/${VERIFY_VERSION}-tauri.json"
    else
      latest_url="${manifest_base}/releases/$STORAGE_CHANNEL/latest.json"
      latest_sig_url="${manifest_base}/releases/$STORAGE_CHANNEL/latest.json.sig"
      latest_tauri_url="${manifest_base}/releases/$STORAGE_CHANNEL/latest-tauri.json"
    fi
  fi
  # Fetch raw manifests (no updater-specific query params) and only add cache-bust.
  probe_query="cb=${VERIFY_CACHE_BUST_SEED}"
  latest_probe_url="${latest_url}?${probe_query}"
  latest_sig_probe_url="${latest_sig_url}?${probe_query}"
  latest_tauri_probe_url="${latest_tauri_url}?${probe_query}"
  manifest_label="latest manifest"
  signature_label="latest manifest signature"
  tauri_manifest_label="latest tauri manifest"
  if [[ -n "$VERIFY_VERSION" ]]; then
    manifest_label="versioned manifest (${VERIFY_VERSION}.json)"
    signature_label="versioned manifest signature (${VERIFY_VERSION}.json.sig)"
    tauri_manifest_label="versioned tauri manifest (${VERIFY_VERSION}-tauri.json)"
  fi
  echo "Fetching $manifest_label: $latest_probe_url"
  if ! fetch_with_retry "$latest_probe_url" "$tmp" "$manifest_label" "$VERIFY_BASE_DISCOVERY_RETRIES"; then
    echo "warn: base candidate failed for $manifest_label: $candidate" >&2
    continue
  fi
  echo "Fetching $signature_label: $latest_sig_probe_url"
  if ! fetch_with_retry "$latest_sig_probe_url" "$tmp_sig" "$signature_label" "$VERIFY_BASE_DISCOVERY_RETRIES"; then
    echo "warn: base candidate failed for $signature_label: $candidate" >&2
    continue
  fi
  echo "Fetching $tauri_manifest_label: $latest_tauri_probe_url"
  if ! fetch_with_retry "$latest_tauri_probe_url" "$tmp_tauri" "$tauri_manifest_label" "$VERIFY_BASE_DISCOVERY_RETRIES"; then
    echo "warn: base candidate failed for $tauri_manifest_label: $candidate" >&2
    continue
  fi
  BASE="$download_base"
  break
done

if [[ -z "$BASE" ]]; then
  echo "error: unable to fetch release manifests from any base candidate" >&2
  printf 'candidates:\n' >&2
  for candidate in "${CANDIDATE_BASES[@]}"; do
    printf '  - %s\n' "$candidate" >&2
  done
  exit 2
fi
echo "Using release verify base: $BASE"

echo "Verifying daemon-facing release manifest signature..."
node core/scripts/verify_release_manifest_signature.cjs "$tmp" "$tmp_sig"

schema_out="$(
  MANIFEST_PATH="$tmp" EXPECTED_PLATFORMS="$EXPECTED_PLATFORMS" VERIFY_SCOPE="$VERIFY_SCOPE" EXPECTED_SOURCE_COMMIT="$RELEASE_SOURCE_COMMIT" EXPECTED_CHANNEL="$CHANNEL" DOWNLOAD_CHANNEL="$STORAGE_CHANNEL" node - <<'NODE'
const fs = require("fs");

function fail(msg) {
  console.error("error:", msg);
  process.exit(1);
}

function isArtifact(value) {
  if (!value || typeof value !== "object") return false;
  if (typeof value.url_path !== "string" || value.url_path.trim().length === 0) return false;
  if (typeof value.sha256 !== "string" || !/^[0-9a-fA-F]{64}$/.test(value.sha256.trim())) return false;
  if (
    Object.prototype.hasOwnProperty.call(value, "size_bytes") &&
    (!Number.isSafeInteger(value.size_bytes) || value.size_bytes <= 0)
  ) {
    return false;
  }
  return true;
}

const manifestPath = process.env.MANIFEST_PATH;
if (!manifestPath) fail("internal: MANIFEST_PATH missing");

const raw = fs.readFileSync(manifestPath, "utf8");
const data = JSON.parse(raw);
const expectedSourceCommit = String(process.env.EXPECTED_SOURCE_COMMIT || "").trim();
const expectedChannel = String(process.env.EXPECTED_CHANNEL || "").trim();
const downloadChannel = String(process.env.DOWNLOAD_CHANNEL || "").trim();

if (!data.channel || !data.latest_version || !data.published_at || !data.source_commit) fail("missing required top-level fields");
if (expectedChannel && String(data.channel || "").trim() !== expectedChannel) {
  fail(`manifest channel mismatch: expected ${expectedChannel}, got ${String(data.channel || "").trim() || "<missing>"}`);
}
if (!data.platforms || typeof data.platforms !== "object") fail("missing platforms");
if (expectedSourceCommit && String(data.source_commit || "").trim() !== expectedSourceCommit) {
  fail(`manifest source_commit mismatch: expected ${expectedSourceCommit}, got ${String(data.source_commit || "").trim() || "<missing>"}`);
}

const platforms = Object.keys(data.platforms);
if (platforms.length === 0) fail("no platform entries");

const expected = String(process.env.EXPECTED_PLATFORMS || "")
  .split(",")
  .map((x) => x.trim())
  .filter(Boolean);
const scope = String(process.env.VERIFY_SCOPE || "all").trim();
if (scope !== "all" && scope !== "expected") {
  fail(`invalid VERIFY_SCOPE: ${scope}`);
}
for (const p of expected) {
  if (!platforms.includes(p)) fail(`expected platform missing from manifest: ${p}`);
}
if (scope === "expected" && expected.length === 0) {
  fail("VERIFY_SCOPE=expected requires RELEASE_EXPECTED_PLATFORMS");
}
const platformsToCheck = scope === "expected" ? [...new Set(expected)] : platforms;

const desktopKeys = ["desktop", "appimage", "deb", "dmg", "msi", "nsis", "exe", "zip"];
const allKeys = [...desktopKeys, "daemon"];
const versionPrefix = `/download/${downloadChannel || data.channel}/${data.latest_version}/`;
const artifactPaths = [];

for (const p of platformsToCheck) {
  const ent = data.platforms[p];
  if (!ent || typeof ent !== "object") fail(`platform ${p} entry is not an object`);

  if (!isArtifact(ent.daemon)) {
    fail(`platform ${p} missing daemon artifact`);
  }
  let desktopCount = 0;
  for (const key of desktopKeys) {
    if (isArtifact(ent[key])) desktopCount += 1;
  }
  if (desktopCount === 0) {
    fail(`platform ${p} missing desktop installer artifact`);
  }
  if (/^linux-(x64|arm64)$/.test(p)) {
    for (const key of ["desktop", "appimage"]) {
      if (!isArtifact(ent[key])) {
        fail(`platform ${p} missing required Linux ${key} artifact`);
      }
    }
  }

  for (const key of allKeys) {
    const artifact = ent[key];
    if (!isArtifact(artifact)) continue;
    if (!artifact.url_path.startsWith(versionPrefix)) {
      fail(`platform ${p} artifact ${key} points to wrong version path: ${artifact.url_path}`);
    }
    artifactPaths.push(`${p}|${key}|${artifact.url_path}|${artifact.sha256.trim()}`);
  }
}

console.log(`manifest schema: OK ${data.latest_version}`);
process.stdout.write(`${artifactPaths.join("\n")}\n`);
NODE
)"

schema_head="$(printf '%s\n' "$schema_out" | sed -n '1p')"
echo "$schema_head"
verified_release_version="${schema_head#manifest schema: OK }"
if [[ -z "$verified_release_version" || "$verified_release_version" == "$schema_head" ]]; then
  echo "error: failed to resolve verified release version from manifest schema output" >&2
  exit 3
fi
artifact_rows="$(printf '%s\n' "$schema_out" | sed -n '2,$p')"

echo "Checking Edge redirect behavior (HEAD)..."
while IFS='|' read -r platform kind url_path sha256; do
  [[ -z "${platform:-}" || -z "${kind:-}" || -z "${url_path:-}" ]] && continue
  url="${BASE}${url_path}"
  code="$(head_redirect_code_with_retry "$url" "${platform}/${kind}")" || exit 3
  echo "- redirect OK: ${platform}/${kind} (${code})"
  if [[ "$kind" == "dmg" ]]; then
    validate_remote_macos_dmg_artifact "$url" "${platform}/${kind}" || exit 3
    echo "- DMG integrity OK: ${platform}/${kind}"
  fi
  case "${platform}:${kind}" in
    linux-x64:daemon|linux-arm64:daemon)
      validate_linux_daemon_artifact "$url" "$sha256" "$platform" || exit 3
      ;;
    linux-x64:appimage|linux-arm64:appimage)
      validate_linux_appimage_artifact "$url" "$sha256" "$platform" || exit 3
      ;;
  esac
done <<<"$artifact_rows"

tauri_schema_out="$(
  MANIFEST_PATH="$tmp_tauri" CHANNEL="$CHANNEL" DOWNLOAD_CHANNEL="$STORAGE_CHANNEL" EXPECTED_PLATFORMS="$EXPECTED_PLATFORMS" VERIFY_SCOPE="$VERIFY_SCOPE" \
  DOWNLOAD_BASE="$BASE" VERIFY_ALLOW_HTTP_UPDATER_URLS="$VERIFY_ALLOW_HTTP_UPDATER_URLS" node - <<'NODE'
const fs = require("fs");

function fail(msg) {
  console.error("error:", msg);
  process.exit(1);
}

const downloadBase = String(process.env.DOWNLOAD_BASE || "").trim().replace(/\/+$/, "");
if (!downloadBase) fail("internal: DOWNLOAD_BASE missing");
const allowHttpUpdaterUrls = String(process.env.VERIFY_ALLOW_HTTP_UPDATER_URLS || "0").trim() === "1";

function canonicalUpdaterUrl(value) {
  try {
    const parsed = new URL(String(value || "").trim());
    const protocolOk =
      parsed.protocol === "https:" || (allowHttpUpdaterUrls && parsed.protocol === "http:");
    if (!protocolOk) return null;
    return `${parsed.origin}${parsed.pathname}`;
  } catch {
    return null;
  }
}

function isUpdaterEntry(value) {
  return Boolean(
    value &&
      typeof value === "object" &&
      typeof value.url === "string" &&
      value.url.trim().length > 0 &&
      canonicalUpdaterUrl(value.url) !== null &&
      typeof value.signature === "string" &&
      value.signature.trim().length > 0,
  );
}

const manifestPath = process.env.MANIFEST_PATH;
if (!manifestPath) fail("internal: MANIFEST_PATH missing");
const channel = String(process.env.CHANNEL || "").trim();
if (!channel) fail("internal: CHANNEL missing");
const downloadChannel = String(process.env.DOWNLOAD_CHANNEL || "").trim() || channel;

const raw = fs.readFileSync(manifestPath, "utf8");
const data = JSON.parse(raw);

if (!data.version || !data.pub_date || !data.source_commit) fail("missing required top-level fields in latest-tauri.json");
if (!data.platforms || typeof data.platforms !== "object") fail("missing platforms in latest-tauri.json");

const platforms = Object.keys(data.platforms);
if (platforms.length === 0) fail("no updater platform entries");

const expected = String(process.env.EXPECTED_PLATFORMS || "")
  .split(",")
  .map((x) => x.trim())
  .filter(Boolean);
const scope = String(process.env.VERIFY_SCOPE || "all").trim();
if (scope !== "all" && scope !== "expected") {
  fail(`invalid VERIFY_SCOPE: ${scope}`);
}
for (const p of expected) {
  if (!platforms.includes(p)) fail(`expected platform missing from latest-tauri.json: ${p}`);
}
if (scope === "expected" && expected.length === 0) {
  fail("VERIFY_SCOPE=expected requires RELEASE_EXPECTED_PLATFORMS");
}
const platformsToCheck = scope === "expected" ? [...new Set(expected)] : platforms;
const updaterPlatformsToCheck = new Set(platformsToCheck);
for (const platform of platformsToCheck) {
  if (/^linux-(x64|arm64)$/.test(platform)) {
    updaterPlatformsToCheck.add(`${platform}-appimage`);
  }
}

const versionPrefix = `${downloadBase}/download/${downloadChannel}/${data.version}/`;
const urls = [];
for (const p of updaterPlatformsToCheck) {
  const ent = data.platforms[p];
  if (!isUpdaterEntry(ent)) {
    fail(`platform ${p} updater entry is invalid`);
  }
  const canonical = canonicalUpdaterUrl(ent.url);
  if (!canonical || !canonical.startsWith(versionPrefix)) {
    fail(`platform ${p} updater URL points to wrong version path: ${ent.url}`);
  }
  urls.push(`${p}|${canonical}`);
}

console.log(`tauri manifest schema: OK ${data.version}`);
process.stdout.write(`${urls.join("\n")}\n`);
NODE
)"

tauri_head="$(printf '%s\n' "$tauri_schema_out" | sed -n '1p')"
echo "$tauri_head"
tauri_rows="$(printf '%s\n' "$tauri_schema_out" | sed -n '2,$p')"

echo "Checking Edge redirect behavior for updater URLs (HEAD) + real signature verification..."
while IFS='|' read -r platform updater_url; do
  [[ -z "${platform:-}" || -z "${updater_url:-}" ]] && continue
  url="${updater_url}"
  code="$(head_redirect_code_with_retry "$url" "${platform}/updater")" || exit 4
  echo "- redirect OK: ${platform}/updater (${code})"
  if linux_remote_artifact_verification_disabled && [[ "$platform" =~ ^linux-(x64|arm64)(-appimage)?$ ]]; then
    echo "- updater verification skipped: ${platform}/updater (disabled)"
    continue
  fi
  artifact_name="${url##*/}"
  artifact_path="$tmp_updater_dir/$artifact_name"
  validation_log="$tmp_updater_dir/${artifact_name}.verify.log"
  attempt=1
  while (( attempt <= VERIFY_VALIDATION_RETRIES )); do
    rm -f "$artifact_path" "$validation_log"
    fetch_artifact_with_retry "$url" "$artifact_path" "${platform}/updater artifact" || exit 4
    if node core/scripts/updater_contract_validate.cjs "$tmp_tauri" "$platform" "$artifact_path" \
      >"$validation_log" 2>&1; then
      cat "$validation_log"
      echo "- updater verify OK: ${platform}/updater (${artifact_name})"
      break
    fi
    cat "$validation_log" >&2 || true
    if (( attempt == VERIFY_VALIDATION_RETRIES )); then
      echo "error: updater verification failed for ${platform} after ${VERIFY_VALIDATION_RETRIES} validation attempt(s)" >&2
      exit 4
    fi
    echo "warn: updater verification failed for ${platform} attempt ${attempt}/${VERIFY_VALIDATION_RETRIES}; retrying..." >&2
    sleep "$VERIFY_RETRY_SLEEP_SECONDS"
    attempt=$((attempt + 1))
  done
done <<<"$tauri_rows"

version="$(
  MANIFEST_PATH="$tmp" node - <<'NODE'
const fs = require("fs");
const data = JSON.parse(fs.readFileSync(process.env.MANIFEST_PATH, "utf8"));
process.stdout.write(String(data.latest_version || ""));
NODE
)"

tauri_version="$(
  MANIFEST_PATH="$tmp_tauri" node - <<'NODE'
const fs = require("fs");
const data = JSON.parse(fs.readFileSync(process.env.MANIFEST_PATH, "utf8"));
process.stdout.write(String(data.version || ""));
NODE
)"

source_commit="$(
  MANIFEST_PATH="$tmp" node - <<'NODE'
const fs = require("fs");
const data = JSON.parse(fs.readFileSync(process.env.MANIFEST_PATH, "utf8"));
process.stdout.write(String(data.source_commit || ""));
NODE
)"

tauri_source_commit="$(
  MANIFEST_PATH="$tmp_tauri" node - <<'NODE'
const fs = require("fs");
const data = JSON.parse(fs.readFileSync(process.env.MANIFEST_PATH, "utf8"));
process.stdout.write(String(data.source_commit || ""));
NODE
)"

if [[ "$version" != "$tauri_version" ]]; then
  if [[ "$VERIFY_SCOPE" == "all" ]]; then
    echo "error: manifest version mismatch (latest.json=$version, latest-tauri.json=$tauri_version)" >&2
    exit 5
  fi
  echo "warn: manifest version mismatch during scoped verification (latest.json=$version, latest-tauri.json=$tauri_version)"
fi
if [[ "$source_commit" != "$tauri_source_commit" ]]; then
  if [[ "$VERIFY_SCOPE" == "all" ]]; then
    echo "error: manifest source_commit mismatch (latest.json=$source_commit, latest-tauri.json=$tauri_source_commit)" >&2
    exit 5
  fi
  echo "warn: manifest source_commit mismatch during scoped verification (latest.json=$source_commit, latest-tauri.json=$tauri_source_commit)"
fi
if [[ "$source_commit" != "$RELEASE_SOURCE_COMMIT" ]]; then
  echo "error: manifest source_commit mismatch (expected $RELEASE_SOURCE_COMMIT, got latest.json=$source_commit)" >&2
  exit 5
fi
if [[ "$tauri_source_commit" != "$RELEASE_SOURCE_COMMIT" ]]; then
  echo "error: manifest source_commit mismatch (expected $RELEASE_SOURCE_COMMIT, got latest-tauri.json=$tauri_source_commit)" >&2
  exit 5
fi

if [[ -n "$VERIFY_VERSION" ]]; then
  if [[ "$version" != "$VERIFY_VERSION" ]]; then
    echo "error: manifest version mismatch (expected $VERIFY_VERSION, got latest.json=$version)" >&2
    exit 5
  fi
  if [[ "$tauri_version" != "$VERIFY_VERSION" ]]; then
    echo "error: manifest version mismatch (expected $VERIFY_VERSION, got latest-tauri.json=$tauri_version)" >&2
    exit 5
  fi
fi

if [[ -n "$VERIFIED_MANIFEST_PATH" ]]; then
  mkdir -p "$(dirname "$VERIFIED_MANIFEST_PATH")"
  mkdir -p "$(dirname "$VERIFIED_MANIFEST_SIG_PATH")"
  mkdir -p "$(dirname "$VERIFIED_TAURI_MANIFEST_PATH")"
  cp -f "$tmp" "$VERIFIED_MANIFEST_PATH"
  cp -f "$tmp_sig" "$VERIFIED_MANIFEST_SIG_PATH"
  cp -f "$tmp_tauri" "$VERIFIED_TAURI_MANIFEST_PATH"
fi

echo "OK: release verify passed for $CHANNEL@$version ($source_commit) via $STORAGE_CHANNEL"
