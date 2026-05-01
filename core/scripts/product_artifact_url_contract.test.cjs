const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const publicArtifactOrigin = "https://api.ctx.rs";
const rawSupabaseProjectHost = /https:\/\/[a-z0-9]+\.supabase\.co/i;

const productFiles = [
  "core/apps/desktop/src-tauri/bundles/runtime_lock.v2.json",
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

function readRepoFile(relativePath) {
  return fs.readFileSync(path.join(repoRoot, relativePath), "utf8");
}

function collectProviderArtifactUrls() {
  const matrix = JSON.parse(readRepoFile("core/crates/ctx-provider-accounts/src/provider_matrix.json"));
  const urls = [];
  for (const provider of matrix.providers || []) {
    const targets = provider?.managed_install?.targets || {};
    for (const [targetKey, target] of Object.entries(targets)) {
      const url = String(target?.url || "").trim();
      if (url.includes("/storage/v1/object/public/")) {
        urls.push({ label: `${provider.id}:${targetKey}`, url });
      }
    }
  }
  return urls;
}

function collectRuntimeArtifactUrls() {
  const lock = JSON.parse(readRepoFile("core/apps/desktop/src-tauri/bundles/runtime_lock.v2.json"));
  const urls = [];
  for (const component of lock.components || []) {
    for (const source of component?.sources || []) {
      const uri = String(source?.uri || "").trim();
      if (uri.includes("/storage/v1/object/public/")) {
        urls.push({ label: `${component.kind}:${component.id}:${component.os}/${component.arch}`, url: uri });
      }
    }
  }
  return urls;
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

test("checked-in provider and runtime storage URLs use the ctx custom domain", () => {
  const urls = [...collectProviderArtifactUrls(), ...collectRuntimeArtifactUrls()];
  assert.ok(urls.length > 0, "expected checked-in provider/runtime artifact URLs");
  const offenders = urls.filter((entry) => !entry.url.startsWith(`${publicArtifactOrigin}/storage/v1/object/public/`));
  assert.deepEqual(offenders, []);
});
