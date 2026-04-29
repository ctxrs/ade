const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const repoRoot = path.resolve(__dirname, "..", "..");
const moduleFile = fs.readFileSync(path.join(repoRoot, "MODULE.bazel"), "utf8");
const buildFile = fs.readFileSync(path.join(repoRoot, "core", "apps", "web", "BUILD.bazel"), "utf8");
const e2eBuildFile = fs.readFileSync(path.join(repoRoot, "core", "apps", "web", "e2e", "BUILD.bazel"), "utf8");
const e2eMacroFile = fs.readFileSync(path.join(repoRoot, "core", "apps", "web", "e2e", "web_e2e_test.bzl"), "utf8");
const coreBuildFile = fs.readFileSync(path.join(repoRoot, "core", "BUILD.bazel"), "utf8");
const verifyAgentRemote = fs.readFileSync(path.join(repoRoot, "core", "scripts", "verify_agent_remote.cjs"), "utf8");
const { WEB_SMOKE_BAZEL_TARGETS } = require("./lib/web_smoke_bazel_targets.cjs");

function targetBlock(name) {
  const lines = buildFile.split(/\r?\n/u);
  const nameLineIndex = lines.findIndex((line) => line.trim() === `name = "${name}",`);
  assert.notEqual(nameLineIndex, -1, `expected target ${name} in core/apps/web/BUILD.bazel`);

  let startIndex = nameLineIndex;
  while (startIndex > 0 && !/^[A-Za-z0-9_.]+\($/u.test(lines[startIndex].trim())) {
    startIndex -= 1;
  }
  assert.match(lines[startIndex].trim(), /^[A-Za-z0-9_.]+\($/u, `expected start of target ${name}`);

  let endIndex = nameLineIndex;
  while (endIndex < lines.length && lines[endIndex].trim() !== ")") {
    endIndex += 1;
  }
  assert.notEqual(endIndex, lines.length, `expected end of target ${name}`);

  return lines.slice(startIndex, endIndex + 1).join("\n");
}

test("supported web validation targets do not use the non-hermetic workspace wrapper", () => {
  for (const targetName of ["unit_smoke", "unit_tests", "lint", "typecheck", "build", "desktop_ipc_corpus"]) {
    const block = targetBlock(targetName);
    assert.doesNotMatch(block, /run_workspace_task\.sh/u, `${targetName} must not use workspace node_modules`);
    assert.doesNotMatch(block, /args = \[[\s\S]*?"pnpm"/u, `${targetName} must not shell out to pnpm`);
    assert.match(block, /HERMETIC_WEB_(?:SMOKE|CHECK)_DATA/u, `${targetName} should use Bazel-managed web inputs`);
  }
});

test("browser e2e targets route through the dedicated Bazel runtime instead of workspace wrappers", () => {
  assert.doesNotMatch(buildFile, /name = "e2e_(premerge|release|cross_platform|visual|soak|load)"/u);
  assert.match(e2eMacroFile, /srcs = \["\/\/core\/apps\/web:scripts\/run-e2e-bazel-runtime\.sh"\]/u);
  assert.match(e2eMacroFile, /tags = \[\s*"local",\s*"no-remote",\s*\]/u);
  for (const targetName of ["premerge_required", "release_required", "cross_platform", "visual", "soak", "load"]) {
    assert.match(e2eBuildFile, new RegExp(`name = "${targetName}"`));
  }
  assert.doesNotMatch(e2eBuildFile, /run_workspace_task\.sh/u);
});

test("rust crate universe is pinned to the checked-in Cargo lock for browser e2e analysis", () => {
  const fromSpecsBlock = moduleFile.match(/crate\.from_specs\(\n[\s\S]*?\n\)/u);
  assert.ok(fromSpecsBlock, "MODULE.bazel should configure crate.from_specs");
  assert.match(fromSpecsBlock[0], /cargo_lockfile = "\/\/core:Cargo\.Bazel\.Cargo\.lock"/u);
  assert.match(fromSpecsBlock[0], /lockfile = "\/\/core:Cargo\.Bazel\.lock"/u);
  assert.ok(coreBuildFile.includes('"Cargo.Bazel.Cargo.lock"'));
  assert.ok(coreBuildFile.includes('"Cargo.Bazel.lock"'));
  assert.ok(coreBuildFile.includes('"Cargo.lock"'));
});

test("playwright browser runtimes are Bazel-owned inputs instead of ambient cache state", () => {
  for (const [targetName, repoName] of [
    ["playwright_browsers_mac15_arm64", "playwright_browser_runtime_mac15_arm64"],
    ["playwright_browsers_ubuntu22_04_x64", "playwright_browser_runtime_ubuntu22_04_x64"],
  ]) {
    const block = targetBlock(targetName);
    assert.ok(
      block.includes(`@${repoName}//:runtime_manifest.json`),
      `${targetName} should consume the Bazel-provided runtime manifest`,
    );
    assert.ok(
      block.includes(`@${repoName}//:runtime_trees`),
      `${targetName} should consume the Bazel-provided extracted runtime trees`,
    );
    assert.doesNotMatch(block, /run_workspace_task\.sh/u);
    assert.doesNotMatch(block, /pnpm/u);
  }
});

test("playwright runtime script tests use a dedicated node:test runner instead of app Vitest wiring", () => {
  const block = targetBlock("playwright_runtime_script_tests");
  assert.ok(
    block.includes('entry_point = "scripts/run-node-test-suite.mjs"'),
    "playwright_runtime_script_tests should use the dedicated node:test runner",
  );
  assert.doesNotMatch(block, /vitest/u);
  assert.doesNotMatch(block, /HERMETIC_WEB_CHECK_DATA/u);
});

test("verify:agent-remote web profiles run Bazel web targets without dependency hydration", () => {
  assert.deepEqual(WEB_SMOKE_BAZEL_TARGETS, [
    "//core/packages/session-supervisor-core:unit_tests",
    "//core/packages/session-thread-layout:unit_smoke",
    "//core/apps/web:unit_smoke",
  ]);
  assert.match(verifyAgentRemote, /"web-smoke"[\s\S]*?targets: WEB_SMOKE_BAZEL_TARGETS/u);
  assert.match(verifyAgentRemote, /"web-unit"[\s\S]*?\/\/core\/packages\/session-supervisor-core:unit_tests/u);
  assert.match(verifyAgentRemote, /"web-unit"[\s\S]*?\/\/core\/apps\/web:unit_tests_non_pretext/u);
  assert.match(verifyAgentRemote, /"web-unit"[\s\S]*?\/\/core\/packages\/session-thread-layout:unit_tests/u);
  assert.match(verifyAgentRemote, /"web-unit"[\s\S]*?\/\/core\/apps\/web:pretext_measurement_unit_tests/u);
  assert.match(verifyAgentRemote, /function requiresWebHydration\(step\) \{\n  return false;\n\}/u);
  assert.doesNotMatch(verifyAgentRemote, /pnpm[^\n]+install[^\n]+frozen-lockfile/u);
});
