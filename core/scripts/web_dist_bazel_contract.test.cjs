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

function targetBlock(buildText, name) {
  const lines = buildText.split(/\r?\n/u);
  const nameLineIndex = lines.findIndex((line) => line.trim() === `name = "${name}",`);
  assert.notEqual(nameLineIndex, -1, `expected target ${name}`);

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
  assert.match(bazelIgnore, /^core\/packages\/session-supervisor-core\/node_modules$/m);
  assert.match(bazelIgnore, /^core\/packages\/session-thread-layout\/node_modules$/m);
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
  const nonPretextBlock = targetBlock(buildFile, "unit_tests_non_pretext");
  const nonPretextFoundationBlock = targetBlock(buildFile, "unit_tests_non_pretext_foundation");
  const nonPretextFoundationSharedBlock = targetBlock(buildFile, "unit_tests_non_pretext_foundation_shared");
  const nonPretextFoundationSharedApiBlock = targetBlock(
    buildFile,
    "unit_tests_non_pretext_foundation_shared_api",
  );
  const nonPretextFoundationSharedSupportBlock = targetBlock(
    buildFile,
    "unit_tests_non_pretext_foundation_shared_support",
  );
  const nonPretextFoundationSharedUtilsBlock = targetBlock(
    buildFile,
    "unit_tests_non_pretext_foundation_shared_utils",
  );
  const nonPretextFoundationSharedUtilsAnalyticsBlock = targetBlock(
    buildFile,
    "unit_tests_non_pretext_foundation_shared_utils_analytics",
  );
  const nonPretextFoundationStateBlock = targetBlock(buildFile, "unit_tests_non_pretext_foundation_state");
  const nonPretextMiscBlock = targetBlock(buildFile, "unit_tests_non_pretext_misc");
  const nonPretextSettingsSetupBlock = targetBlock(buildFile, "unit_tests_non_pretext_settings_setup");
  const nonPretextWorkbenchBlock = targetBlock(buildFile, "unit_tests_non_pretext_workbench");
  const nonPretextWorkbenchSurfaceBlock = targetBlock(buildFile, "unit_tests_non_pretext_workbench_surface");
  const nonPretextWorkbenchSurfaceAppBlock = targetBlock(buildFile, "unit_tests_non_pretext_workbench_surface_app");
  const nonPretextWorkbenchSurfaceSessionBlock = targetBlock(buildFile, "unit_tests_non_pretext_workbench_surface_session");
  const nonPretextWorkbenchSurfaceSessionCoreBlock = targetBlock(
    buildFile,
    "unit_tests_non_pretext_workbench_surface_session_core",
  );
  const nonPretextWorkbenchSurfaceSessionThreadBlock = targetBlock(
    buildFile,
    "unit_tests_non_pretext_workbench_surface_session_thread",
  );
  const nonPretextWorkbenchShellBlock = targetBlock(buildFile, "unit_tests_non_pretext_workbench_shell");
  const pretextBlock = targetBlock(buildFile, "pretext_measurement_unit_tests");
  const desktopIpcCorpusBlock = targetBlock(buildFile, "desktop_ipc_corpus");

  assert.equal(
    corePackageJson.scripts["bazel:web:unit:supervisor-core"],
    "node scripts/run_bazel_pilot.cjs test //core/packages/session-supervisor-core:unit_tests",
  );
  assert.equal(
    corePackageJson.scripts["bazel:web:unit:thread-layout"],
    "node scripts/run_bazel_pilot.cjs test //core/packages/session-thread-layout:unit_tests",
  );
  assert.equal(
    corePackageJson.scripts["bazel:web:unit:non-pretext"],
    "node scripts/run_bazel_pilot.cjs test //core/apps/web:unit_tests_non_pretext",
  );
  assert.equal(
    corePackageJson.scripts["bazel:web:unit:non-pretext:foundation"],
    "node scripts/run_bazel_pilot.cjs test //core/apps/web:unit_tests_non_pretext_foundation",
  );
  assert.equal(
    corePackageJson.scripts["bazel:web:unit:non-pretext:foundation:shared"],
    [
      "node scripts/run_bazel_pilot.cjs test //core/apps/web:unit_tests_non_pretext_foundation_shared_api",
      "node scripts/run_bazel_pilot.cjs test //core/apps/web:unit_tests_non_pretext_foundation_shared_support",
      "node scripts/run_bazel_pilot.cjs test //core/apps/web:unit_tests_non_pretext_foundation_shared_utils",
      "node scripts/run_bazel_pilot.cjs test //core/apps/web:unit_tests_non_pretext_foundation_shared_utils_analytics",
    ].join(" && "),
  );
  assert.equal(
    corePackageJson.scripts["bazel:web:unit:non-pretext:foundation:state"],
    "node scripts/run_bazel_pilot.cjs test //core/apps/web:unit_tests_non_pretext_foundation_state",
  );
  assert.equal(
    corePackageJson.scripts["bazel:web:unit:non-pretext:settings-setup"],
    "node scripts/run_bazel_pilot.cjs test //core/apps/web:unit_tests_non_pretext_settings_setup",
  );
  assert.equal(
    corePackageJson.scripts["bazel:web:unit:non-pretext:workbench"],
    "node scripts/run_bazel_pilot.cjs test //core/apps/web:unit_tests_non_pretext_workbench",
  );
  assert.equal(
    corePackageJson.scripts["bazel:web:unit:non-pretext:workbench:surface"],
    "node scripts/run_bazel_pilot.cjs test //core/apps/web:unit_tests_non_pretext_workbench_surface",
  );
  assert.equal(
    corePackageJson.scripts["bazel:web:unit:non-pretext:workbench:surface:app"],
    "node scripts/run_bazel_pilot.cjs test //core/apps/web:unit_tests_non_pretext_workbench_surface_app",
  );
  assert.equal(
    corePackageJson.scripts["bazel:web:unit:non-pretext:workbench:surface:session"],
    "node scripts/run_bazel_pilot.cjs test //core/apps/web:unit_tests_non_pretext_workbench_surface_session",
  );
  assert.equal(
    corePackageJson.scripts["bazel:web:unit:non-pretext:workbench:shell"],
    "node scripts/run_bazel_pilot.cjs test //core/apps/web:unit_tests_non_pretext_workbench_shell",
  );
  assert.equal(
    corePackageJson.scripts["bazel:web:unit:non-pretext:misc"],
    "node scripts/run_bazel_pilot.cjs test //core/apps/web:unit_tests_non_pretext_misc",
  );
  assert.equal(
    corePackageJson.scripts["bazel:web:pretext:measurement"],
    "node scripts/run_bazel_pilot.cjs test //core/packages/session-thread-layout:unit_tests //core/apps/web:pretext_measurement_unit_tests",
  );
  assert.match(nonPretextBlock, /^test_suite\(/m);
  assert.match(nonPretextFoundationBlock, /^test_suite\(/m);
  assert.match(nonPretextFoundationSharedBlock, /^test_suite\(/m);
  assert.match(nonPretextFoundationSharedApiBlock, /^vitest_bin\.vitest_test\(/m);
  assert.match(nonPretextFoundationSharedSupportBlock, /^vitest_bin\.vitest_test\(/m);
  assert.match(nonPretextFoundationSharedUtilsBlock, /^vitest_bin\.vitest_test\(/m);
  assert.match(nonPretextFoundationSharedUtilsAnalyticsBlock, /^vitest_bin\.vitest_test\(/m);
  assert.match(nonPretextFoundationStateBlock, /^vitest_bin\.vitest_test\(/m);
  assert.match(nonPretextMiscBlock, /^vitest_bin\.vitest_test\(/m);
  assert.match(nonPretextSettingsSetupBlock, /^vitest_bin\.vitest_test\(/m);
  assert.match(nonPretextWorkbenchBlock, /^test_suite\(/m);
  assert.match(nonPretextWorkbenchSurfaceBlock, /^test_suite\(/m);
  assert.match(nonPretextWorkbenchSurfaceAppBlock, /^vitest_bin\.vitest_test\(/m);
  assert.match(nonPretextWorkbenchSurfaceSessionBlock, /^test_suite\(/m);
  assert.match(nonPretextWorkbenchSurfaceSessionCoreBlock, /^vitest_bin\.vitest_test\(/m);
  assert.match(nonPretextWorkbenchSurfaceSessionThreadBlock, /^vitest_bin\.vitest_test\(/m);
  assert.match(nonPretextWorkbenchShellBlock, /^vitest_bin\.vitest_test\(/m);
  assert.match(pretextBlock, /^vitest_bin\.vitest_test\(/m);
  assert.match(desktopIpcCorpusBlock, /^vitest_bin\.vitest_test\(/m);
  assert.match(nonPretextBlock, /unit_tests_non_pretext_foundation/);
  assert.match(nonPretextBlock, /unit_tests_non_pretext_misc/);
  assert.match(nonPretextBlock, /unit_tests_non_pretext_settings_setup/);
  assert.match(nonPretextBlock, /unit_tests_non_pretext_workbench/);
  assert.match(nonPretextFoundationBlock, /unit_tests_non_pretext_foundation_shared/);
  assert.match(nonPretextFoundationBlock, /unit_tests_non_pretext_foundation_state/);
  assert.match(nonPretextFoundationSharedBlock, /unit_tests_non_pretext_foundation_shared_api/);
  assert.match(nonPretextFoundationSharedBlock, /unit_tests_non_pretext_foundation_shared_support/);
  assert.match(nonPretextFoundationSharedBlock, /unit_tests_non_pretext_foundation_shared_utils/);
  assert.match(nonPretextFoundationSharedBlock, /unit_tests_non_pretext_foundation_shared_utils_analytics/);
  assert.match(nonPretextWorkbenchSurfaceBlock, /unit_tests_non_pretext_workbench_surface_app/);
  assert.match(nonPretextWorkbenchSurfaceBlock, /unit_tests_non_pretext_workbench_surface_session/);
  assert.match(nonPretextWorkbenchSurfaceSessionBlock, /unit_tests_non_pretext_workbench_surface_session_core/);
  assert.match(nonPretextWorkbenchSurfaceSessionBlock, /unit_tests_non_pretext_workbench_surface_session_thread/);
  assert.match(nonPretextWorkbenchBlock, /unit_tests_non_pretext_workbench_surface/);
  assert.match(nonPretextWorkbenchBlock, /unit_tests_non_pretext_workbench_shell/);
  assert.match(nonPretextFoundationSharedApiBlock, /data = HERMETIC_WEB_CHECK_DATA/);
  assert.match(nonPretextFoundationSharedSupportBlock, /data = HERMETIC_WEB_CHECK_DATA/);
  assert.match(nonPretextFoundationSharedUtilsBlock, /data = HERMETIC_WEB_CHECK_DATA/);
  assert.match(nonPretextFoundationSharedUtilsAnalyticsBlock, /data = HERMETIC_WEB_CHECK_DATA/);
  assert.match(nonPretextFoundationSharedApiBlock, /timeout = "long"/);
  assert.match(nonPretextFoundationSharedSupportBlock, /timeout = "long"/);
  assert.match(nonPretextFoundationSharedUtilsBlock, /timeout = "long"/);
  assert.match(nonPretextFoundationSharedUtilsAnalyticsBlock, /timeout = "long"/);
  assert.match(nonPretextFoundationStateBlock, /data = HERMETIC_WEB_CHECK_DATA/);
  assert.match(nonPretextMiscBlock, /data = HERMETIC_WEB_CHECK_DATA/);
  assert.match(nonPretextSettingsSetupBlock, /data = HERMETIC_WEB_CHECK_DATA/);
  assert.match(nonPretextWorkbenchSurfaceAppBlock, /data = HERMETIC_WEB_CHECK_DATA/);
  assert.match(nonPretextWorkbenchSurfaceSessionCoreBlock, /data = HERMETIC_WEB_CHECK_DATA/);
  assert.match(nonPretextWorkbenchSurfaceSessionThreadBlock, /data = HERMETIC_WEB_CHECK_DATA/);
  assert.match(nonPretextWorkbenchShellBlock, /data = HERMETIC_WEB_CHECK_DATA/);
  assert.match(pretextBlock, /data = HERMETIC_WEB_PRETEXT_APP_DATA/);
  assert.match(pretextBlock, /src\/pages\/sessionThread\/transcriptLayoutPlanner\.appSmoke\.test\.ts/);
  assert.match(desktopIpcCorpusBlock, /data = HERMETIC_WEB_CHECK_DATA/);
  assert.doesNotMatch(nonPretextBlock, /run_workspace_task\.sh/);
  assert.doesNotMatch(nonPretextFoundationBlock, /run_workspace_task\.sh/);
  assert.doesNotMatch(nonPretextFoundationSharedBlock, /run_workspace_task\.sh/);
  assert.doesNotMatch(nonPretextFoundationStateBlock, /run_workspace_task\.sh/);
  assert.doesNotMatch(nonPretextMiscBlock, /run_workspace_task\.sh/);
  assert.doesNotMatch(nonPretextSettingsSetupBlock, /run_workspace_task\.sh/);
  assert.doesNotMatch(nonPretextWorkbenchBlock, /run_workspace_task\.sh/);
  assert.doesNotMatch(nonPretextWorkbenchSurfaceSessionCoreBlock, /run_workspace_task\.sh/);
  assert.doesNotMatch(nonPretextWorkbenchSurfaceSessionThreadBlock, /run_workspace_task\.sh/);
  assert.doesNotMatch(pretextBlock, /run_workspace_task\.sh/);
  assert.doesNotMatch(desktopIpcCorpusBlock, /run_workspace_task\.sh/);
  assert.doesNotMatch(desktopIpcCorpusBlock, /\bpnpm\b/);
  assert.match(
    buildFile,
    /NON_PRETEXT_WORKBENCH_SURFACE_APP_TESTS = glob\(\[[\s\S]*?"\*\.test\.ts"[\s\S]*?"\*\.test\.tsx"/,
  );
  assert.match(
    buildFile,
    /NON_PRETEXT_WORKBENCH_SURFACE_SESSION_CORE_TESTS = glob\(\[[\s\S]*?"src\/pages\/SessionPage\*\.test\.ts"[\s\S]*?"src\/pages\/sessionView\/\*\*\/\*\.test\.tsx"/,
  );
  assert.match(
    buildFile,
    /NON_PRETEXT_WORKBENCH_SURFACE_SESSION_THREAD_TESTS = glob\(\[[\s\S]*?"src\/pages\/sessionThread\/\*\.test\.ts"[\s\S]*?"src\/pages\/sessionThread\/\*\*\/\*\.test\.tsx"/,
  );
  assert.match(
    buildFile,
    /NON_PRETEXT_WORKBENCH_SHELL_TESTS = glob\(\[[\s\S]*?"src\/pages\/workbenchShell\/\*\.test\.ts"[\s\S]*?"src\/pages\/workbenchShell\/\*\*\/\*\.test\.tsx"/,
  );
  assert.match(
    buildFile,
    /NON_PRETEXT_MISC_ARGS = NON_PRETEXT_VITEST_ARGS \+ \[[\s\S]*?"--exclude",\s*"\*\.test\.ts"[\s\S]*?"--exclude",\s*"\*\.test\.tsx"/,
  );
  assert.match(
    buildFile,
    /NON_PRETEXT_MISC_ARGS = NON_PRETEXT_VITEST_ARGS \+ \[[\s\S]*?"--exclude",\s*"src\/testdata\/\*\.test\.ts"[\s\S]*?"--exclude",\s*"src\/testdata\/\*\*\/\*\.test\.tsx"/,
  );
  assert.match(buildFile, /NON_PRETEXT_EXCLUDES = \[[\s\S]*?"src\/pages\/sessionThread\/pretext\*\.test\.ts"/);
  assert.match(buildFile, /NON_PRETEXT_EXCLUDES = \[[\s\S]*?"src\/pages\/sessionThread\/sessionMarkdown\*\.test\.ts"/);
});
