const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

test("bundled harness image helper reads DEFAULT_CONTAINER_IMAGE from the sandbox container runtime crate", () => {
  const scriptPath = path.resolve(__dirname, "..", "..", "scripts", "lib", "bundled_harnesses_images.sh");
  const text = fs.readFileSync(scriptPath, "utf8");

  assert.match(text, /ctx-sandbox-container-runtime\/src\/lib\.rs/);
  assert.doesNotMatch(text, /ctx-http\/src\/workspace_runtime\/mod\.rs/);
});
