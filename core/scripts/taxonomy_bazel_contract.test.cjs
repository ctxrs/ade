const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(coreRoot, "..");
const packageJson = JSON.parse(fs.readFileSync(path.join(coreRoot, "package.json"), "utf8"));
const rootBuild = fs.readFileSync(path.join(repoRoot, "BUILD.bazel"), "utf8");
const coreBuild = fs.readFileSync(path.join(coreRoot, "BUILD.bazel"), "utf8");
const ctxHttpBuild = fs.readFileSync(path.join(coreRoot, "crates", "ctx-http", "BUILD.bazel"), "utf8");
const ctxProviderAccountsBuild = fs.readFileSync(
  path.join(coreRoot, "crates", "ctx-provider-accounts", "BUILD.bazel"),
  "utf8",
);
const scriptsBuild = fs.readFileSync(path.join(coreRoot, "scripts", "BUILD.bazel"), "utf8");

test("deterministic Linux contract gates route through Bazel-owned workspace-task wrappers", () => {
  assert.equal(
    packageJson.scripts["bazel:provider-auth:validate"],
    "node scripts/run_bazel_pilot.cjs run //core/scripts:provider_auth_validate",
  );
  assert.equal(
    packageJson.scripts["bazel:desktop:check:versions"],
    "node scripts/run_bazel_pilot.cjs run //core/scripts:desktop_version_check",
  );
  assert.equal(
    packageJson.scripts["bazel:desktop:runtime:lock:check-matrix"],
    "node scripts/run_bazel_pilot.cjs run //core/scripts:desktop_runtime_lock_check_matrix",
  );
  assert.equal(
    packageJson.scripts["bazel:desktop:runtime:lock:validate"],
    "node scripts/run_bazel_pilot.cjs run //core/scripts:desktop_runtime_lock_validate",
  );
  assert.equal(
    packageJson.scripts["bazel:bundles:codex-archive-artifacts"],
    "node scripts/run_bazel_pilot.cjs run //tools/bazel:codex_archive_artifact_gate",
  );
  assert.equal(
    packageJson.scripts["bazel:bundles:codex-provenance"],
    "node scripts/run_bazel_pilot.cjs run //tools/bazel:codex_provenance_policy",
  );
  assert.equal(
    packageJson.scripts["bazel:linux:bundle:gate"],
    "node scripts/run_bazel_pilot.cjs run //tools/bazel:linux_bundle_contracts -- --platform linux-x64 --bundles-dir core/apps/desktop/src-tauri/bundles",
  );
  assert.equal(
    packageJson.scripts["bazel:web:e2e:release:retry"],
    "bash -lc '../scripts/ci_retry.sh pnpm bazel:web:e2e:release'",
  );
  assert.equal(
    packageJson.scripts["bazel:desktop:provider-matrix:contracts"],
    "node scripts/run_bazel_pilot.cjs run //core/scripts:desktop_provider_matrix_archive_contracts",
  );
  assert.equal(
    packageJson.scripts["bazel:desktop:launch-mode:contracts"],
    "node scripts/run_bazel_pilot.cjs run //core/scripts:desktop_launch_mode_contracts",
  );
  assert.equal(
    packageJson.scripts["bazel:desktop:bundle:contracts"],
    "node scripts/run_bazel_pilot.cjs run //core/scripts:desktop_bundle_contracts",
  );
  assert.equal(
    packageJson.scripts["bazel:bundled-harness:dependency:contracts"],
    "node scripts/run_bazel_pilot.cjs run //core/scripts:bundled_harness_dependency_contracts",
  );
  assert.equal(
    packageJson.scripts["bazel:desktop:e2e:preflight:contracts"],
    "node scripts/run_bazel_pilot.cjs run //core/scripts:desktop_e2e_preflight_contracts",
  );
  assert.equal(
    packageJson.scripts["bazel:desktop:sync-resources:contracts"],
    "node scripts/run_bazel_pilot.cjs run //core/scripts:desktop_sync_resources_contracts",
  );
  assert.equal(
    packageJson.scripts["bazel:providers:e2e:bundle:contracts"],
    "node scripts/run_bazel_pilot.cjs run //core/scripts:providers_e2e_bundle_contracts",
  );
  assert.equal(
    packageJson.scripts["bazel:providers:linux-arm:contracts"],
    "node scripts/run_bazel_pilot.cjs run //core/scripts:linux_arm_provider_contracts",
  );
  assert.equal(
    packageJson.scripts["bazel:tauri:tools:lock:contracts"],
    "node scripts/run_bazel_pilot.cjs run //core/scripts:tauri_tools_lock_contracts",
  );
  assert.equal(
    packageJson.scripts["bazel:desktop:deps:contracts"],
    "node scripts/run_bazel_pilot.cjs run //core/scripts:install_desktop_deps_contracts",
  );

  for (const filegroup of [
    'name = "buildkite_pipeline_sources"',
    'name = "buildkite_script_sources"',
    'name = "ci_script_sources"',
    'name = "repo_script_sources"',
  ]) {
    assert.match(rootBuild, new RegExp(filegroup.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&")));
  }

  for (const filegroup of [
    'name = "desktop_automation_files"',
    'name = "desktop_release_metadata_files"',
    'name = "provider_matrix_sources"',
  ]) {
    assert.match(coreBuild, new RegExp(filegroup.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&")));
  }
  assert.match(coreBuild, /exports_files\(\[\s*"Cargo\.lock",\s*"Cargo\.toml",\s*"package\.json",\s*"pnpm-lock\.yaml",\s*"pnpm-workspace\.yaml",\s*\]\)/s);
  assert.match(coreBuild, /"\/\/core\/crates\/ctx-http:Cargo\.toml"/);
  assert.match(coreBuild, /"\/\/core\/crates\/ctx-provider-accounts:src\/provider_matrix\.json"/);
  assert.match(ctxHttpBuild, /exports_files\(\["Cargo\.toml"\]\)/);
  assert.match(ctxProviderAccountsBuild, /exports_files\(\["src\/provider_matrix\.json"\]\)/);

  for (const target of [
    'name = "provider_auth_validate"',
    'name = "desktop_version_check"',
    'name = "desktop_runtime_lock_check_matrix"',
    'name = "desktop_runtime_lock_validate"',
    'name = "desktop_provider_matrix_archive_contracts"',
    'name = "desktop_launch_mode_contracts"',
    'name = "desktop_bundle_contracts"',
    'name = "bundled_harness_dependency_contracts"',
    'name = "desktop_e2e_preflight_contracts"',
    'name = "desktop_sync_resources_contracts"',
    'name = "providers_e2e_bundle_contracts"',
    'name = "linux_arm_provider_contracts"',
    'name = "tauri_tools_lock_contracts"',
    'name = "install_desktop_deps_contracts"',
  ]) {
    assert.match(scriptsBuild, new RegExp(target.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&")));
  }

  assert.match(scriptsBuild, /srcs = \["\/\/tools\/bazel:run_workspace_task\.sh"\]/);
  assert.match(
    scriptsBuild,
    /args = \["core", "node", "--test", "scripts\/buildkite_pipeline_contract\.test\.cjs", "scripts\/trigger_buildkite_mac_nightly\.test\.cjs"\]/,
  );
  assert.match(scriptsBuild, /args = \["core", "pnpm", "provider-auth:validate"\]/);
  assert.match(scriptsBuild, /args = \["core", "pnpm", "desktop:check:versions"\]/);
  assert.match(scriptsBuild, /args = \["core", "pnpm", "desktop:runtime:lock:check-matrix"\]/);
  assert.match(scriptsBuild, /args = \["core", "node", "scripts\/runtime_lock_validate\.cjs", "--profile", "parity"\]/);
});
