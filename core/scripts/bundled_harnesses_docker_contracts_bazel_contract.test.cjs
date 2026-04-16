const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const buildFilePath = path.resolve(__dirname, "..", "..", "tools", "bazel", "BUILD.bazel");
const buildFileText = fs.readFileSync(buildFilePath, "utf8");
const scriptPath = path.resolve(__dirname, "..", "..", "tools", "bazel", "bundled_harnesses_docker_contracts.sh");
const scriptText = fs.readFileSync(scriptPath, "utf8");

test("bundled harness docker contracts Bazel entrypoint stays explicit", () => {
  assert.match(buildFileText, /name = "bundled_harnesses_docker_contracts"/);
  assert.match(scriptText, /BUILD_WORKSPACE_DIRECTORY/);
  assert.match(scriptText, /ensure_bundled_harnesses_docker_only\.sh/);
});
