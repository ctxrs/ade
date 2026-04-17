const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const buildFile = fs.readFileSync(path.join(repoRoot, "core", "apps", "web", "BUILD.bazel"), "utf8");
const scriptText = fs.readFileSync(path.join(repoRoot, "core", "apps", "web", "dist_sync_tool.sh"), "utf8");

test("Bazel web dist sync target stays explicit about the caller-supplied output dir", () => {
  assert.match(buildFile, /name = "dist_sync"/);
  assert.match(buildFile, /srcs = \["dist_sync_tool\.sh"\]/);
});

test("Bazel web dist sync tool requires an explicit workspace root and output dir", () => {
  assert.match(scriptText, /usage: \$0 <output-dir>/);
  assert.match(scriptText, /BUILD_WORKSPACE_DIRECTORY is required/);
  assert.doesNotMatch(scriptText, /\brsync\b/);
  assert.match(scriptText, /core\/apps\/desktop\/src-tauri\/bin/);
  assert.match(scriptText, /core\/apps\/desktop\/src-tauri\/bundles/);
  assert.match(scriptText, /core\/apps\/web\/playwright-report/);
  assert.match(scriptText, /core\/apps\/web\/test-results/);
  assert.match(scriptText, /core\/apps\/web\/node_modules\/\.bin\/vite/);
  assert.match(scriptText, /"\$\{VITE_BIN\}" build/);
  assert.match(scriptText, /cp -R dist "\$\{OUTPUT_DIR\}"/);
});
