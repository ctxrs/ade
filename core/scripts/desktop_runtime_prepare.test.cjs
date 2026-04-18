#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  buildPrepareEnvSnapshot,
  buildPrepareFingerprint,
  canReusePreparedParity,
} = require("./desktop_runtime_prepare.cjs");

function writeFixtureFile(rootDir, relativePath, contents = "") {
  const absolutePath = path.join(rootDir, relativePath);
  fs.mkdirSync(path.dirname(absolutePath), { recursive: true });
  fs.writeFileSync(absolutePath, contents, "utf8");
  return absolutePath;
}

function createPrepareFixture() {
  const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-runtime-prepare-"));
  const coreRoot = path.join(rootDir, "core");
  fs.mkdirSync(coreRoot, { recursive: true });
  writeFixtureFile(coreRoot, "package.json", "{\n  \"name\": \"core\"\n}\n");
  writeFixtureFile(coreRoot, "pnpm-lock.yaml", "lockfileVersion: '9.0'\n");
  writeFixtureFile(coreRoot, "apps/desktop/package.json", "{\n  \"name\": \"desktop\"\n}\n");
  writeFixtureFile(coreRoot, "apps/desktop/src-tauri/Cargo.toml", "[package]\nname = \"desktop\"\nversion = \"0.1.0\"\n");
  writeFixtureFile(coreRoot, "apps/desktop/src-tauri/src/main.rs", "fn main() {}\n");
  writeFixtureFile(coreRoot, "crates/ctx-http/src/lib.rs", "pub fn ctx_http() {}\n");
  writeFixtureFile(coreRoot, "crates/ctx-mcp/src/lib.rs", "pub fn ctx_mcp() {}\n");
  writeFixtureFile(
    coreRoot,
    "crates/ctx-sandbox-container-runtime/src/lib.rs",
    "pub const DEFAULT_CONTAINER_IMAGE: &str = \"ctx-harness:latest\";\n",
  );
  writeFixtureFile(coreRoot, "crates/ctx-provider-accounts/src/provider_matrix.json", "{\n  \"cells\": []\n}\n");
  writeFixtureFile(coreRoot, "scripts/desktop_check_versions.cjs", "console.log('ok');\n");
  writeFixtureFile(coreRoot, "scripts/desktop_sync_resources.cjs", "console.log('sync');\n");
  writeFixtureFile(coreRoot, "scripts/desktop_sync_resources_remote_daemon_policy.cjs", "module.exports = {};\n");
  writeFixtureFile(coreRoot, "scripts/runtime_lock_validate.cjs", "module.exports = {};\n");
  writeFixtureFile(coreRoot, "scripts/lib/web_dist_cache.cjs", "module.exports = {};\n");
  writeFixtureFile(coreRoot, "scripts/prepare_avf_linux_guest_runtime.sh", "#!/usr/bin/env bash\n");
  writeFixtureFile(rootDir, "scripts/ensure_bundled_harnesses.sh", "#!/usr/bin/env bash\n");
  const lockPath = writeFixtureFile(coreRoot, "apps/desktop/src-tauri/bundles/runtime_lock.v2.json", "{\n  \"version\": 2\n}\n");
  const overridesPath = writeFixtureFile(rootDir, ".ctx/local/runtime_overrides.json", "{\n  \"overrides\": []\n}\n");
  return { coreRoot, lockPath, overridesPath, rootDir };
}

test("desktop runtime prepare fingerprint changes when tracked inputs change", () => {
  const { coreRoot, lockPath, overridesPath, rootDir } = createPrepareFixture();
  const env = { HOME: rootDir };

  const initial = buildPrepareFingerprint({
    coreRoot,
    env,
    profile: "parity",
    desktopVersion: "0.22.0",
    lockPath,
    overridesPath,
    cargoTargetDir: "/tmp/cargo-target",
    computeWebDistCacheKeyImpl: () => "cached-web-dist-key",
  });

  writeFixtureFile(coreRoot, "crates/ctx-http/src/lib.rs", "pub fn ctx_http() { println!(\"changed\"); }\n");

  const updated = buildPrepareFingerprint({
    coreRoot,
    env,
    profile: "parity",
    desktopVersion: "0.22.0",
    lockPath,
    overridesPath,
    cargoTargetDir: "/tmp/cargo-target",
    computeWebDistCacheKeyImpl: () => "cached-web-dist-key",
  });

  assert.notEqual(initial.fingerprint, updated.fingerprint);
});

test("desktop runtime prepare reuses parity prep only when fingerprint and outputs match", () => {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-runtime-reuse-"));
  const requiredOutputs = [
    path.join(tempRoot, "manifest.json"),
    path.join(tempRoot, "runtime_manifest.effective.json"),
  ];
  for (const outputPath of requiredOutputs) {
    writeFixtureFile(tempRoot, path.relative(tempRoot, outputPath), "ok\n");
  }

  assert.equal(
    canReusePreparedParity({
      profile: "parity",
      state: {
        version: 3,
        profile: "parity",
        prepare: {
          fingerprint_version: 1,
          fingerprint: "abc123",
        },
      },
      fingerprint: "abc123",
      requiredOutputs,
    }),
    true,
  );

  fs.rmSync(requiredOutputs[0], { force: true });
  assert.equal(
    canReusePreparedParity({
      profile: "parity",
      state: {
        version: 3,
        profile: "parity",
        prepare: {
          fingerprint_version: 1,
          fingerprint: "abc123",
        },
      },
      fingerprint: "abc123",
      requiredOutputs,
    }),
    false,
  );

  assert.equal(
    canReusePreparedParity({
      profile: "source-all",
      state: {
        version: 3,
        profile: "source-all",
        prepare: {
          fingerprint_version: 1,
          fingerprint: "abc123",
        },
      },
      fingerprint: "abc123",
      requiredOutputs: [],
    }),
    false,
  );
});

test("desktop runtime prepare env snapshot keeps only runtime-relevant keys", () => {
  assert.deepEqual(
    buildPrepareEnvSnapshot({
      CTX_BUNDLE_REMOTE_DAEMONS: "0",
      CTX_RUNTIME_PROFILE: "parity",
      CTX_DESKTOP_WEB_DIST: "apps/web/dist",
      SOMETHING_ELSE: "ignore-me",
    }),
    {
      CTX_BUNDLE_REMOTE_DAEMONS: "0",
      CTX_DESKTOP_WEB_DIST: "apps/web/dist",
      CTX_RUNTIME_PROFILE: "parity",
    },
  );
});
