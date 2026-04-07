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
  assert.equal(passThroughEnv.has("RUSTUP_HOME"), true);
});
