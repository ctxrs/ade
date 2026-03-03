#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

tmp_root="$(mktemp -d /tmp/ctx-release-manifest-race.XXXXXX)"
trap 'rm -rf "$tmp_root"' EXIT

channel="stable"
version="9.9.9"
download_base="https://example.test/functions/v1"
latest_json="$tmp_root/latest.json"
latest_tauri_json="$tmp_root/latest-tauri.json"
lock_dir="$tmp_root/manifest.lock"

cat >"$latest_json" <<'JSON'
{
  "channel": "stable",
  "latest_version": "8.8.8",
  "published_at": "2026-02-10T00:00:00Z",
  "platforms": {
    "linux-x64": {
      "desktop": {
        "url_path": "/download/stable/8.8.8/ctx_8.8.8_linux-x64",
        "sha256": "stale"
      },
      "daemon": {
        "url_path": "/download/stable/8.8.8/ctx_8.8.8_linux-x64-daemon",
        "sha256": "stale"
      }
    }
  }
}
JSON

cat >"$latest_tauri_json" <<'JSON'
{
  "version": "8.8.8",
  "notes": "ctx 8.8.8",
  "pub_date": "2026-02-10T00:00:00Z",
  "platforms": {
    "linux-x64": {
      "url": "https://example.test/functions/v1/download/stable/8.8.8/ctx_8.8.8_linux-x64_updater.AppImage.tar.gz",
      "signature": "stale"
    }
  }
}
JSON

platforms=(
  "linux-x64"
  "linux-arm64"
  "macos-x64"
  "macos-arm64"
  "windows-x64"
)

acquire_lock() {
  local deadline
  deadline=$((SECONDS + 20))
  while ! mkdir "$lock_dir" >/dev/null 2>&1; do
    if (( SECONDS >= deadline )); then
      echo "error: timed out waiting for manifest test lock" >&2
      exit 1
    fi
    sleep 0.02
  done
}

release_lock() {
  rmdir "$lock_dir" >/dev/null 2>&1 || true
}

merge_platform_once() {
  local platform="$1"
  local desktop_name="ctx_${version}_${platform}"
  local daemon_name="$desktop_name"
  local updater_name="ctx_${version}_${platform}_updater.tar.gz"
  if [[ "$platform" == "windows-x64" ]]; then
    daemon_name="${daemon_name}.exe"
  fi

  local platform_entry_json
  platform_entry_json="$(
    CHANNEL="$channel" VERSION="$version" PLATFORM="$platform" DESKTOP_NAME="$desktop_name" DAEMON_NAME="$daemon_name" node - <<'NODE'
const e = process.env;
process.stdout.write(
  JSON.stringify({
    desktop: {
      url_path: `/download/${e.CHANNEL}/${e.VERSION}/${e.DESKTOP_NAME}`,
      sha256: `desktop-${e.PLATFORM}`,
    },
    daemon: {
      url_path: `/download/${e.CHANNEL}/${e.VERSION}/${e.DAEMON_NAME}`,
      sha256: `daemon-${e.PLATFORM}`,
    },
  }),
);
NODE
  )"

  local tauri_entry_json
  tauri_entry_json="$(
    CHANNEL="$channel" VERSION="$version" PLATFORM="$platform" UPDATER_NAME="$updater_name" DOWNLOAD_BASE="$download_base" node - <<'NODE'
const e = process.env;
const downloadBase = String(e.DOWNLOAD_BASE || "").replace(/\/+$/, "");
process.stdout.write(
  JSON.stringify({
    url: `${downloadBase}/download/${e.CHANNEL}/${e.VERSION}/${e.UPDATER_NAME}`,
    signature: `sig-${e.PLATFORM}`,
  }),
);
NODE
  )"

  acquire_lock
  CHANNEL="$channel" VERSION="$version" PLATFORM="$platform" PUBLISHED_AT="2026-02-19T00:00:00Z" \
  EXISTING_MANIFEST_FILE="$latest_json" PLATFORM_ENTRY_JSON="$platform_entry_json" \
  node - <<'NODE' >"$latest_json.next"
const fs = require("fs");
const channel = process.env.CHANNEL;
const version = process.env.VERSION;
const platform = process.env.PLATFORM;
const publishedAt = process.env.PUBLISHED_AT;
const existingRaw = fs.readFileSync(process.env.EXISTING_MANIFEST_FILE, "utf8");
const platformEntry = JSON.parse(process.env.PLATFORM_ENTRY_JSON);

const artifactKeys = ["desktop", "appimage", "deb", "dmg", "msi", "nsis", "exe", "zip", "daemon"];
const versionPrefix = `/download/${channel}/${version}/`;
const desktopArtifactKeys = ["desktop", "appimage", "deb", "dmg", "msi", "nsis", "exe", "zip"];

function isArtifact(x) {
  return Boolean(
    x &&
      typeof x === "object" &&
      typeof x.url_path === "string" &&
      x.url_path.trim().length > 0 &&
      typeof x.sha256 === "string" &&
      x.sha256.trim().length > 0,
  );
}

function entryIsCurrentVersion(entry) {
  if (!entry || typeof entry !== "object") return false;
  let found = false;
  for (const key of artifactKeys) {
    const artifact = entry[key];
    if (!isArtifact(artifact)) continue;
    found = true;
    if (!artifact.url_path.startsWith(versionPrefix)) return false;
  }
  return found;
}

if (!isArtifact(platformEntry.daemon)) {
  throw new Error("platform entry missing daemon");
}
if (!desktopArtifactKeys.some((key) => isArtifact(platformEntry[key]))) {
  throw new Error("platform entry missing desktop artifact");
}

let existing = {};
try {
  existing = JSON.parse(existingRaw || "{}");
} catch {
  existing = {};
}
const existingPlatformsRaw =
  existing.platforms && typeof existing.platforms === "object" ? existing.platforms : {};
const filtered = {};
for (const [key, entry] of Object.entries(existingPlatformsRaw)) {
  if (entryIsCurrentVersion(entry)) {
    filtered[key] = entry;
  }
}
const merged = {
  channel,
  latest_version: version,
  published_at: publishedAt,
  platforms: {
    ...filtered,
    [platform]: platformEntry,
  },
};
process.stdout.write(`${JSON.stringify(merged, null, 2)}\n`);
NODE
  mv "$latest_json.next" "$latest_json"

  CHANNEL="$channel" VERSION="$version" PLATFORM="$platform" PUBLISHED_AT="2026-02-19T00:00:00Z" DOWNLOAD_BASE="$download_base" \
  EXISTING_TAURI_MANIFEST_FILE="$latest_tauri_json" TAURI_PLATFORM_ENTRY_JSON="$tauri_entry_json" \
  node - <<'NODE' >"$latest_tauri_json.next"
const fs = require("fs");
const channel = process.env.CHANNEL;
const version = process.env.VERSION;
const platform = process.env.PLATFORM;
const publishedAt = process.env.PUBLISHED_AT;
const existingRaw = fs.readFileSync(process.env.EXISTING_TAURI_MANIFEST_FILE, "utf8");
const platformEntry = JSON.parse(process.env.TAURI_PLATFORM_ENTRY_JSON);
const downloadBase = String(process.env.DOWNLOAD_BASE || "").trim().replace(/\/+$/, "");
const versionPrefix = `${downloadBase}/download/${channel}/${version}/`;

function canonicalUpdaterUrl(raw) {
  try {
    const parsed = new URL(String(raw || "").trim());
    if (parsed.protocol !== "https:") return null;
    return `${parsed.origin}${parsed.pathname}`;
  } catch {
    return null;
  }
}

function isUpdaterEntry(x) {
  return Boolean(
    x &&
      typeof x === "object" &&
      typeof x.url === "string" &&
      x.url.trim().length > 0 &&
      canonicalUpdaterUrl(x.url) !== null &&
      typeof x.signature === "string" &&
      x.signature.trim().length > 0,
  );
}

function entryIsCurrentVersion(entry) {
  if (!isUpdaterEntry(entry)) return false;
  const canonical = canonicalUpdaterUrl(entry.url);
  return Boolean(canonical && canonical.startsWith(versionPrefix));
}

if (!isUpdaterEntry(platformEntry)) {
  throw new Error("platform updater entry missing");
}

let existing = {};
try {
  existing = JSON.parse(existingRaw || "{}");
} catch {
  existing = {};
}

const existingPlatformsRaw =
  existing.platforms && typeof existing.platforms === "object" ? existing.platforms : {};
const filtered = {};
for (const [key, entry] of Object.entries(existingPlatformsRaw)) {
  if (entryIsCurrentVersion(entry)) {
    filtered[key] = entry;
  }
}

const merged = {
  version,
  notes: `ctx ${version}`,
  pub_date: publishedAt,
  platforms: {
    ...filtered,
    [platform]: platformEntry,
  },
};
process.stdout.write(`${JSON.stringify(merged, null, 2)}\n`);
NODE
  mv "$latest_tauri_json.next" "$latest_tauri_json"
  release_lock
}

for platform in "${platforms[@]}"; do
  merge_platform_once "$platform" &
done
wait

LATEST_JSON_PATH="$latest_json" LATEST_TAURI_JSON_PATH="$latest_tauri_json" EXPECTED_PLATFORMS="$(IFS=,; echo "${platforms[*]}")" DOWNLOAD_BASE="$download_base" \
node - <<'NODE'
const fs = require("fs");

const downloadBase = String(process.env.DOWNLOAD_BASE || "").trim().replace(/\/+$/, "");
const expectedPlatforms = String(process.env.EXPECTED_PLATFORMS || "")
  .split(",")
  .map((v) => v.trim())
  .filter(Boolean);
const latest = JSON.parse(fs.readFileSync(process.env.LATEST_JSON_PATH, "utf8"));
const tauri = JSON.parse(fs.readFileSync(process.env.LATEST_TAURI_JSON_PATH, "utf8"));

if (latest.latest_version !== "9.9.9") {
  throw new Error(`unexpected latest.json version: ${latest.latest_version}`);
}
if (tauri.version !== "9.9.9") {
  throw new Error(`unexpected latest-tauri.json version: ${tauri.version}`);
}
if (latest.latest_version !== tauri.version) {
  throw new Error("latest.json and latest-tauri.json versions diverged");
}

for (const platform of expectedPlatforms) {
  if (!latest.platforms || !latest.platforms[platform]) {
    throw new Error(`latest.json missing platform ${platform}`);
  }
  if (!tauri.platforms || !tauri.platforms[platform]) {
    throw new Error(`latest-tauri.json missing platform ${platform}`);
  }
}

if (latest.platforms["linux-x64"]?.desktop?.url_path?.includes("/8.8.8/")) {
  throw new Error("stale prior-version platform artifact survived merge");
}

const expectedUpdaterPrefix = `${downloadBase}/download/stable/9.9.9/`;
for (const platform of expectedPlatforms) {
  const updater = tauri.platforms[platform];
  if (typeof updater.url !== "string" || !updater.url.startsWith(expectedUpdaterPrefix)) {
    throw new Error(`updater URL not absolute/current for ${platform}: ${updater?.url}`);
  }
}

const invalidRelative = {
  ...tauri,
  platforms: {
    ...tauri.platforms,
    "linux-x64": {
      ...tauri.platforms["linux-x64"],
      url: "/download/stable/9.9.9/ctx_9.9.9_linux-x64_updater.AppImage.tar.gz",
    },
  },
};
const rejectedRelative = (() => {
  try {
    const candidate = invalidRelative.platforms["linux-x64"]?.url;
    const parsed = new URL(String(candidate || ""));
    return parsed.protocol !== "https:";
  } catch {
    return true;
  }
})();
if (!rejectedRelative) {
  throw new Error("relative updater URL unexpectedly accepted");
}

console.log("ok: release manifest concurrent merge smoke passed");
NODE
