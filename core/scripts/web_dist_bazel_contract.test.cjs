const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const bazelIgnore = fs.readFileSync(path.join(repoRoot, ".bazelignore"), "utf8");
const buildFile = fs.readFileSync(path.join(repoRoot, "core", "apps", "web", "BUILD.bazel"), "utf8");
const scriptText = fs.readFileSync(path.join(repoRoot, "core", "apps", "web", "dist_sync_tool.sh"), "utf8");
const anyEnforceToolText = fs.readFileSync(path.join(repoRoot, "core", "apps", "web", "any_enforce_tool.sh"), "utf8");
const corePackageJson = JSON.parse(fs.readFileSync(path.join(repoRoot, "core", "package.json"), "utf8"));

function targetBlock(buildText, name, nextName) {
  const start = buildText.indexOf(`name = "${name}"`);
  assert.notEqual(start, -1, `expected target ${name}`);
  const end = buildText.indexOf(`name = "${nextName}"`, start + 1);
  assert.notEqual(end, -1, `expected target ${nextName}`);
  return buildText.slice(start, end);
}

test("Bazel web dist sync target stays explicit about the caller-supplied output dir", () => {
  assert.match(buildFile, /name = "dist_sync"/);
  assert.match(buildFile, /srcs = \["dist_sync_tool\.sh"\]/);
  assert.match(buildFile, /":node_modules\/@ctx\/design"/);
  assert.match(buildFile, /":node_modules\/@ctx\/types"/);
  assert.match(buildFile, /":node_modules\/@pretext-virtualizer\/core"/);
  assert.match(buildFile, /":node_modules\/@pretext-virtualizer\/interface"/);
});

test("Bazel ignores workspace-local node_modules trees that would collide with importer aliases", () => {
  assert.match(bazelIgnore, /^core\/apps\/web\/node_modules$/m);
  assert.match(bazelIgnore, /^core\/apps\/desktop\/node_modules$/m);
  assert.match(bazelIgnore, /^core\/packages\/ctx-design\/node_modules$/m);
  assert.match(bazelIgnore, /^core\/packages\/ctx-types\/node_modules$/m);
  assert.match(bazelIgnore, /^core\/packages\/pretext-virtualizer-core\/node_modules$/m);
  assert.match(bazelIgnore, /^core\/packages\/pretext-virtualizer-interface\/node_modules$/m);
  assert.match(bazelIgnore, /^core\/packages\/web-session-worker\/node_modules$/m);
});

test("Bazel web any-enforce target stays on a lightweight data set", () => {
  assert.match(buildFile, /ANY_ENFORCE_DATA = WEB_SOURCES \+ \[/);
  assert.match(buildFile, /name = "any_enforce"[\s\S]*?srcs = \["any_enforce_tool\.sh"\][\s\S]*?data = ANY_ENFORCE_DATA/);

  const anyEnforceDataBlock = buildFile.match(/ANY_ENFORCE_DATA = WEB_SOURCES \+ \[(.*?)\n\]/s);
  assert.ok(anyEnforceDataBlock, "expected ANY_ENFORCE_DATA block");
  assert.doesNotMatch(anyEnforceDataBlock[1], /desktop_bundle_files/);
  assert.doesNotMatch(anyEnforceDataBlock[1], /pnpm-lock\.yaml/);
  assert.doesNotMatch(anyEnforceDataBlock[1], /pnpm-workspace\.yaml/);
});

test("Bazel web any-enforce tool runs directly from runfiles with node", () => {
  assert.match(anyEnforceToolText, /MODULE\.bazel/);
  assert.match(anyEnforceToolText, /core\/package\.json/);
  assert.match(anyEnforceToolText, /resolve_node/);
  assert.match(anyEnforceToolText, /explicit-any-report\.mjs --enforce --enforce-mode zero --format table/);
  assert.doesNotMatch(anyEnforceToolText, /run_workspace_task/);
  assert.doesNotMatch(anyEnforceToolText, /\bpnpm\b/);
});

test("Bazel web dist sync tool requires an explicit workspace root and output dir", () => {
  assert.match(scriptText, /usage: \$0 <output-dir>/);
  assert.match(scriptText, /BUILD_WORKSPACE_DIRECTORY is required/);
  assert.doesNotMatch(scriptText, /\brsync\b/);
  assert.doesNotMatch(scriptText, /core\/apps\/desktop\/src-tauri/);
  assert.match(scriptText, /prepare_minimal_workspace/);
  assert.match(scriptText, /core\/package\.json/);
  assert.match(scriptText, /core\/pnpm-lock\.yaml/);
  assert.match(scriptText, /core\/pnpm-workspace\.yaml/);
  assert.match(scriptText, /core\/packages/);
  assert.match(scriptText, /dist\|node_modules\|coverage\|playwright-report\|test-results/);
  assert.match(scriptText, /find "\$\{runfiles_root\}\/core\/apps\/web" -mindepth 1 -maxdepth 1/);
  assert.match(scriptText, /core\/node_modules/);
  assert.match(scriptText, /core\/apps\/web\/node_modules/);
  assert.match(scriptText, /expected web-local vite at/);
  assert.match(scriptText, /"\$\{VITE_BIN\}" build/);
  assert.match(scriptText, /cp -R dist "\$\{OUTPUT_DIR\}"/);
});

test("Bazel web focused unit slices run as native vitest tests", () => {
  const nonPretextBlock = targetBlock(buildFile, "unit_tests_non_pretext", "pretext_measurement_unit_tests");
  const pretextBlock = targetBlock(buildFile, "pretext_measurement_unit_tests", "desktop_ipc_corpus");

  assert.equal(
    corePackageJson.scripts["bazel:web:unit:non-pretext"],
    "node scripts/run_bazel_pilot.cjs test //core/apps/web:unit_tests_non_pretext",
  );
  assert.equal(
    corePackageJson.scripts["bazel:web:pretext:measurement"],
    "node scripts/run_bazel_pilot.cjs test //core/apps/web:pretext_measurement_unit_tests",
  );
  assert.match(buildFile, /vitest_bin\.vitest_test\([\s\S]*?name = "unit_tests_non_pretext"/);
  assert.match(buildFile, /vitest_bin\.vitest_test\([\s\S]*?name = "pretext_measurement_unit_tests"/);
  assert.match(nonPretextBlock, /data = HERMETIC_WEB_CHECK_DATA/);
  assert.match(pretextBlock, /data = HERMETIC_WEB_CHECK_DATA/);
  assert.doesNotMatch(nonPretextBlock, /run_workspace_task\.sh/);
  assert.doesNotMatch(pretextBlock, /run_workspace_task\.sh/);
});
