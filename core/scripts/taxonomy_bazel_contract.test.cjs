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

function readTargetBlock(targetName) {
  const start = scriptsBuild.indexOf(`name = "${targetName}"`);
  assert.notEqual(start, -1, `missing target block for ${targetName}`);
  const next = scriptsBuild.indexOf("\nsh_binary(", start + 1);
  return scriptsBuild.slice(start, next === -1 ? undefined : next);
}

test("deterministic Linux contract gates route through Bazel-owned contract entrypoints", () => {
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


  const providerAuthValidate = readTargetBlock("provider_auth_validate");
  assert.match(providerAuthValidate, /srcs = \["\/\/tools\/bazel:run_node_task\.sh"\]/);
  assert.match(providerAuthValidate, /args = \["core", "scripts\/provider_auth_validate_bazel\.cjs"\]/);
  assert.doesNotMatch(providerAuthValidate, /run_workspace_task\.sh/);
  assert.doesNotMatch(providerAuthValidate, /\bpnpm\b/);

  const desktopVersionCheck = readTargetBlock("desktop_version_check");
  assert.match(desktopVersionCheck, /srcs = \["\/\/tools\/bazel:run_node_task\.sh"\]/);
  assert.match(desktopVersionCheck, /args = \["core", "scripts\/desktop_check_versions\.cjs"\]/);
  assert.doesNotMatch(desktopVersionCheck, /run_workspace_task\.sh/);
  assert.doesNotMatch(desktopVersionCheck, /\bpnpm\b/);

  const runtimeLockCheckMatrix = readTargetBlock("desktop_runtime_lock_check_matrix");
  assert.match(runtimeLockCheckMatrix, /srcs = \["\/\/tools\/bazel:run_node_task\.sh"\]/);
  assert.match(runtimeLockCheckMatrix, /args = \["core", "scripts\/runtime_lock_matrix_consistency\.cjs"\]/);
  assert.doesNotMatch(runtimeLockCheckMatrix, /run_workspace_task\.sh/);
  assert.doesNotMatch(runtimeLockCheckMatrix, /\bpnpm\b/);

  const runtimeLockValidate = readTargetBlock("desktop_runtime_lock_validate");
  assert.match(runtimeLockValidate, /srcs = \["\/\/tools\/bazel:run_node_task\.sh"\]/);
  assert.match(runtimeLockValidate, /args = \["core", "scripts\/runtime_lock_validate\.cjs", "--profile", "parity"\]/);
  assert.doesNotMatch(runtimeLockValidate, /run_workspace_task\.sh/);

  const launchModeContracts = readTargetBlock("desktop_launch_mode_contracts");
  assert.match(launchModeContracts, /srcs = \["\/\/tools\/bazel:run_node_task\.sh"\]/);
  assert.match(
    launchModeContracts,
    /args = \["core", "--test", "scripts\/desktop_mode_contract\.test\.cjs", "scripts\/desktop_prepare\.test\.cjs", "scripts\/desktop_tauri_entry\.test\.cjs"\]/,
  );
  assert.doesNotMatch(launchModeContracts, /run_workspace_task\.sh/);

  const bundleContracts = readTargetBlock("desktop_bundle_contracts");
  assert.match(bundleContracts, /srcs = \["\/\/tools\/bazel:run_node_task\.sh"\]/);
  assert.match(
    bundleContracts,
    /args = \["core", "--test", "scripts\/desktop_import_bundles\.test\.cjs", "scripts\/desktop_normalize_bundle_permissions\.test\.cjs", "scripts\/desktop_icon_reps\.test\.cjs"\]/,
  );
  assert.doesNotMatch(bundleContracts, /run_workspace_task\.sh/);

  const providerMatrixArchiveContracts = readTargetBlock("desktop_provider_matrix_archive_contracts");
  assert.match(providerMatrixArchiveContracts, /srcs = \["\/\/tools\/bazel:run_node_task\.sh"\]/);
  assert.match(
    providerMatrixArchiveContracts,
    /args = \["core", "--test", "scripts\/provider_matrix_archive_gap_report\.test\.cjs", "scripts\/provider_matrix_required_targets_gate\.test\.cjs"\]/,
  );
  assert.doesNotMatch(providerMatrixArchiveContracts, /run_workspace_task\.sh/);

  const desktopE2EPreflightContracts = readTargetBlock("desktop_e2e_preflight_contracts");
  assert.match(desktopE2EPreflightContracts, /srcs = \["\/\/tools\/bazel:run_node_task\.sh"\]/);
  assert.match(
    desktopE2EPreflightContracts,
    /args = \["core", "--test", "scripts\/desktop_e2e_preflight\.test\.cjs"\]/,
  );
  assert.doesNotMatch(desktopE2EPreflightContracts, /run_workspace_task\.sh/);

  const desktopSyncResourcesContracts = readTargetBlock("desktop_sync_resources_contracts");
  assert.match(desktopSyncResourcesContracts, /srcs = \["\/\/tools\/bazel:run_node_task\.sh"\]/);
  assert.match(
    desktopSyncResourcesContracts,
    /args = \["core", "--test", "scripts\/desktop_sync_resources_remote_daemon_policy\.test\.cjs", "scripts\/desktop_sync_resources_avf_guest_runtime\.test\.cjs", "scripts\/desktop_daemon_sidecar_name\.test\.cjs", "apps\/desktop\/automation\/wdio\.container-assets\.test\.cjs"\]/,
  );
  assert.doesNotMatch(desktopSyncResourcesContracts, /run_workspace_task\.sh/);

  const providersE2EBundleContracts = readTargetBlock("providers_e2e_bundle_contracts");
  assert.match(providersE2EBundleContracts, /srcs = \["\/\/tools\/bazel:run_node_task\.sh"\]/);
  assert.match(
    providersE2EBundleContracts,
    /args = \["core", "--test", "scripts\/providers_e2e_bundle_isolation\.test\.cjs"\]/,
  );
  assert.doesNotMatch(providersE2EBundleContracts, /run_workspace_task\.sh/);

  const linuxArmProviderContracts = readTargetBlock("linux_arm_provider_contracts");
  assert.match(linuxArmProviderContracts, /srcs = \["\/\/tools\/bazel:run_node_task\.sh"\]/);
  assert.match(
    linuxArmProviderContracts,
    /args = \["core", "--test", "scripts\/linux_arm_provider_reliability_matrix\.test\.cjs", "scripts\/linux_arm_provider_preflight\.test\.cjs", "scripts\/linux_arm_provider_report_summary\.test\.cjs", "scripts\/linux_arm_provider_release_gate\.test\.cjs"\]/,
  );
  assert.doesNotMatch(linuxArmProviderContracts, /run_workspace_task\.sh/);


  const tauriToolsLockContracts = readTargetBlock("tauri_tools_lock_contracts");
  assert.match(tauriToolsLockContracts, /srcs = \["\/\/tools\/bazel:run_node_task\.sh"\]/);
  assert.match(
    tauriToolsLockContracts,
    /args = \["core", "--test", "scripts\/tauri_tools_lock_contract\.test\.cjs"\]/,
  );
  assert.doesNotMatch(tauriToolsLockContracts, /run_workspace_task\.sh/);
});
