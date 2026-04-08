const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const turboConfigPath = path.join(__dirname, "..", "turbo.json");
const turboConfig = JSON.parse(fs.readFileSync(turboConfigPath, "utf8"));

test("turbo forwards external Rust cache env vars to child tasks", () => {
  const passThroughEnv = new Set(turboConfig.globalPassThroughEnv ?? []);

  assert.equal(passThroughEnv.has("CARGO_HOME"), true);
  assert.equal(passThroughEnv.has("CARGO_TARGET_DIR"), true);
  assert.equal(passThroughEnv.has("CTX_VERIFY_CARGO_TARGET_DIR"), true);
  assert.equal(passThroughEnv.has("CTX_VOLATILE_ROOT"), true);
  assert.equal(passThroughEnv.has("CTX_VOLATILE_ROOT_MODE"), true);
  assert.equal(passThroughEnv.has("CTX_VOLATILE_TARGETS_DIR"), true);
  assert.equal(passThroughEnv.has("CTX_VOLATILE_ARTIFACTS_DIR"), true);
  assert.equal(passThroughEnv.has("CTX_VOLATILE_TMPDIR"), true);
  assert.equal(passThroughEnv.has("CTX_BUNDLE_CACHE_DIR"), true);
  assert.equal(passThroughEnv.has("PLAYWRIGHT_BROWSERS_PATH"), true);
  assert.equal(passThroughEnv.has("RUSTC_WRAPPER"), true);
  assert.equal(passThroughEnv.has("SCCACHE_DIR"), true);
  assert.equal(passThroughEnv.has("SCCACHE_BASEDIRS"), true);
  assert.equal(passThroughEnv.has("SCCACHE_PATH"), true);
  assert.equal(passThroughEnv.has("SCCACHE_SERVER_UDS"), true);
  assert.equal(passThroughEnv.has("CARGO_INCREMENTAL"), true);
  assert.equal(passThroughEnv.has("RUSTFLAGS"), true);
  assert.equal(passThroughEnv.has("RUST_TEST_THREADS"), true);
  assert.equal(passThroughEnv.has("RUSTUP_HOME"), true);
});
