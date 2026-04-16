const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(coreRoot, "..");
const packageJson = JSON.parse(fs.readFileSync(path.join(coreRoot, "package.json"), "utf8"));
const desktopIpcBuild = fs.readFileSync(path.join(coreRoot, "crates", "ctx-desktop-ipc", "BUILD.bazel"), "utf8");
const desktopIpcGenerator = fs.readFileSync(
  path.join(coreRoot, "crates", "ctx-desktop-ipc", "src", "bin", "generate_typescript.rs"),
  "utf8",
);
const webBuild = fs.readFileSync(path.join(coreRoot, "apps", "web", "BUILD.bazel"), "utf8");

test("desktop IPC check routes through the Bazel-owned freshness target", () => {
  assert.equal(
    packageJson.scripts["desktop:ipc:check"],
    "node scripts/run_bazel_pilot.cjs test //core/crates/ctx-desktop-ipc:typescript_check_test",
  );
  assert.match(desktopIpcBuild, /name = "generate_typescript"/);
  assert.match(desktopIpcBuild, /name = "typescript_check"/);
  assert.match(desktopIpcBuild, /name = "typescript_check_test"/);
  assert.match(desktopIpcBuild, /\$\(location :generate_typescript\)/);
  assert.match(desktopIpcBuild, /\$\(location \/\/core\/apps\/web:desktop_ipc_generated_typescript\)/);
  assert.match(webBuild, /name = "desktop_ipc_generated_typescript"/);
  assert.match(desktopIpcGenerator, /if arg == "--output"/);
  assert.match(desktopIpcGenerator, /missing value for --output/);
});
