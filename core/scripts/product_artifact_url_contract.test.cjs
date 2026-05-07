const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const publicStoragePrefix = "/storage/v1/object/public/releases/";
const providerArtifactPrefix = `${publicStoragePrefix}providers/`;
const executableRuntimePrefixes = [
  `${publicStoragePrefix}artifacts/managed-runtimes/`,
  `${publicStoragePrefix}images/`,
  `${publicStoragePrefix}runtimes/`,
];
const rawSupabaseProjectHost = /https:\/\/[a-z0-9]+\.supabase\.co/i;
const githubReleaseUrl = /https:\/\/github\.com\/[^"'\s]+\/releases\/download\//i;
const nodeRuntimeUrl = /https:\/\/nodejs\.org\//i;
const pythonBuildStandaloneUrl = /https:\/\/github\.com\/indygreg\/python-build-standalone\//i;
const factoryDownloadsUrl = /https:\/\/downloads\.factory\.ai\//i;

const productFiles = [
  "core/apps/desktop/src-tauri/bundles/runtime_lock.v2.json",
  "core/apps/desktop/src-tauri/build.rs",
  "core/crates/ctx-managed-installs/src/title_generation_local.rs",
  "core/crates/ctx-provider-accounts/src/provider_matrix.json",
  "core/scripts/lib/release_bom.cjs",
  "scripts/lib/tauri_tools_lock.mjs",
  "scripts/release_e2e_with_infisical.sh",
  "scripts/release_promote_supabase_latest.sh",
  "scripts/run_mobile_e2e.sh",
  "scripts/supabase_prod_env.sh",
  "supabase/functions/download/index.ts",
  "supabase/functions/provider-matrix/index.ts",
  "supabase/functions/releases/index.ts",
];

const executableArtifactMetadataFiles = [
  "core/apps/desktop/src-tauri/bundles/runtime_lock.v2.json",
  "core/apps/desktop/src-tauri/build.rs",
  "core/crates/ctx-managed-installs/src/title_generation_local.rs",
  "core/crates/ctx-provider-accounts/src/provider_matrix.json",
];

function readRepoFile(relativePath) {
  return fs.readFileSync(path.join(repoRoot, relativePath), "utf8");
}

function collectProviderArtifactUrls() {
  const matrix = JSON.parse(readRepoFile("core/crates/ctx-provider-accounts/src/provider_matrix.json"));
  const urls = [];

  function collectTargets(scopeLabel, targets) {
    for (const [targetKey, target] of Object.entries(targets || {})) {
      const url = String(target?.url || "").trim();
      if (!url) continue;
      urls.push({
        label: `${scopeLabel}:${targetKey}`,
        sha256: String(target?.sha256 || "").trim().toLowerCase(),
        url,
      });
    }
  }

  for (const provider of matrix.providers || []) {
    collectTargets(`${provider.id}:managed`, provider?.managed_install?.targets);
    for (const dependency of provider?.dependencies || []) {
      collectTargets(
        `${provider.id}:dependency:${dependency?.id || "unknown"}`,
        dependency?.install?.targets,
      );
    }
  }
  return urls;
}

function collectRuntimeLockArtifactUrls() {
  const lock = JSON.parse(readRepoFile("core/apps/desktop/src-tauri/bundles/runtime_lock.v2.json"));
  const urls = [];

  function visit(value, trail) {
    if (Array.isArray(value)) {
      value.forEach((entry, index) => visit(entry, [...trail, String(index)]));
      return;
    }
    if (!value || typeof value !== "object") {
      return;
    }
    const uri = String(value.uri || "").trim();
    if (uri.startsWith("http://") || uri.startsWith("https://")) {
      urls.push({
        label: trail.join("."),
        sha256: String(value.sha256 || "").trim().toLowerCase(),
        url: uri,
      });
    }
    for (const [key, nested] of Object.entries(value)) {
      visit(nested, [...trail, key]);
    }
  }

  visit(lock, ["runtime_lock"]);
  return urls;
}

function collectTitleRuntimeUrls() {
  const text = readRepoFile("core/crates/ctx-managed-installs/src/title_generation_local.rs");
  const urls = [];
  const pattern = /url:\s*"([^"]+)",\s*\n\s*sha256:\s*"([0-9a-f]{64})"/g;
  for (const match of text.matchAll(pattern)) {
    urls.push({
      label: `title-generation-runtime:${urls.length}`,
      sha256: match[2].toLowerCase(),
      url: match[1],
    });
  }
  return urls;
}

function collectVoskRuntimeUrls() {
  const text = readRepoFile("core/apps/desktop/src-tauri/build.rs");
  const urls = [];
  const pattern = /"(https:\/\/[^"]*\/runtimes\/vosk\/[^"]+)",\s*\n\s*"([0-9a-f]{64})"/g;
  for (const match of text.matchAll(pattern)) {
    urls.push({
      label: `vosk-runtime:${urls.length}`,
      sha256: match[2].toLowerCase(),
      url: match[1],
    });
  }
  return urls;
}

function parseHttpsUrl(entry) {
  try {
    const parsed = new URL(entry.url);
    if (parsed.protocol !== "https:") {
      return `${entry.label}: URL must use https (${entry.url})`;
    }
    return parsed;
  } catch {
    return `${entry.label}: URL is invalid (${entry.url})`;
  }
}

function hasRawSupabaseHost(parsed) {
  return /^[a-z0-9]+\.supabase\.co$/i.test(parsed.hostname);
}

function hasSlashShaPath(parsed, sha256) {
  return new RegExp(`/sha256/${sha256}(?:/|$)`).test(parsed.pathname);
}

function hasRuntimeShaPath(parsed, sha256) {
  return hasSlashShaPath(parsed, sha256) || new RegExp(`/sha256-${sha256}(?:/|$)`).test(parsed.pathname);
}

function validateCtxArtifactUrls({
  allowedPrefixes,
  entries,
  shaPathKind,
}) {
  const offenders = [];
  for (const entry of entries) {
    const parsed = parseHttpsUrl(entry);
    if (typeof parsed === "string") {
      offenders.push(parsed);
      continue;
    }
    if (parsed.hostname !== "api.ctx.rs") {
      offenders.push(`${entry.label}: URL host must be api.ctx.rs (${entry.url})`);
    }
    if (hasRawSupabaseHost(parsed)) {
      offenders.push(`${entry.label}: URL must not use raw Supabase project host (${entry.url})`);
    }
    if (!allowedPrefixes.some((prefix) => parsed.pathname.startsWith(prefix))) {
      offenders.push(`${entry.label}: URL path is outside allowed artifact prefixes (${entry.url})`);
    }
    if (!/^[0-9a-f]{64}$/.test(entry.sha256)) {
      offenders.push(`${entry.label}: missing/invalid sha256 metadata`);
      continue;
    }
    const hasShaPath = shaPathKind === "runtime"
      ? hasRuntimeShaPath(parsed, entry.sha256)
      : hasSlashShaPath(parsed, entry.sha256);
    if (!hasShaPath) {
      offenders.push(`${entry.label}: URL path must include sha256 ${entry.sha256} (${entry.url})`);
    }
  }
  return offenders;
}

test("product artifact metadata does not expose raw Supabase project hosts", () => {
  const offenders = [];
  for (const relativePath of productFiles) {
    const text = readRepoFile(relativePath);
    if (rawSupabaseProjectHost.test(text)) {
      offenders.push(relativePath);
    }
  }
  assert.deepEqual(offenders, []);
});

test("checked-in executable artifact metadata does not use upstream distribution URLs", () => {
  const offenders = [];
  for (const relativePath of executableArtifactMetadataFiles) {
    const text = readRepoFile(relativePath);
    if (githubReleaseUrl.test(text)) {
      offenders.push(`${relativePath}: GitHub Releases URL`);
    }
    if (nodeRuntimeUrl.test(text)) {
      offenders.push(`${relativePath}: nodejs.org URL`);
    }
    if (pythonBuildStandaloneUrl.test(text)) {
      offenders.push(`${relativePath}: python-build-standalone URL`);
    }
    if (factoryDownloadsUrl.test(text)) {
      offenders.push(`${relativePath}: downloads.factory.ai URL`);
    }
  }
  assert.deepEqual(offenders, []);
});

test("checked-in provider artifact URLs use ctx custom domain and sha-addressed paths", () => {
  const urls = collectProviderArtifactUrls();
  assert.ok(urls.length > 0, "expected checked-in provider artifact URLs");
  assert.deepEqual(
    validateCtxArtifactUrls({
      allowedPrefixes: [providerArtifactPrefix],
      entries: urls,
      shaPathKind: "provider",
    }),
    [],
  );
});

test("checked-in runtime lock artifact URLs use ctx custom domain and sha-addressed paths", () => {
  const urls = collectRuntimeLockArtifactUrls();
  assert.ok(urls.length > 0, "expected checked-in runtime lock artifact URLs");
  assert.deepEqual(
    validateCtxArtifactUrls({
      allowedPrefixes: executableRuntimePrefixes,
      entries: urls,
      shaPathKind: "runtime",
    }),
    [],
  );
});

test("checked-in title-generation runtime URLs use ctx mirror artifacts", () => {
  const urls = collectTitleRuntimeUrls();
  assert.equal(urls.length, 4);
  assert.deepEqual(
    validateCtxArtifactUrls({
      allowedPrefixes: [`${publicStoragePrefix}runtimes/llama.cpp/`],
      entries: urls,
      shaPathKind: "provider",
    }),
    [],
  );
});

test("checked-in Vosk runtime URLs use ctx mirror artifacts", () => {
  const urls = collectVoskRuntimeUrls();
  assert.equal(urls.length, 4);
  assert.deepEqual(
    validateCtxArtifactUrls({
      allowedPrefixes: [`${publicStoragePrefix}runtimes/vosk/`],
      entries: urls,
      shaPathKind: "provider",
    }),
    [],
  );
});
