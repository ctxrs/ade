const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(coreRoot, "..");
const packageJson = JSON.parse(fs.readFileSync(path.join(coreRoot, "package.json"), "utf8"));
const rootBuild = fs.readFileSync(path.join(repoRoot, "BUILD.bazel"), "utf8");
const coreBuild = fs.readFileSync(path.join(coreRoot, "BUILD.bazel"), "utf8");
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

  for (const filegroup of [
    'name = "buildkite_pipeline_sources"',
    'name = "buildkite_script_sources"',
    'name = "ci_script_sources"',
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

  for (const target of [
    'name = "provider_auth_validate"',
    'name = "desktop_version_check"',
    'name = "desktop_runtime_lock_check_matrix"',
    'name = "desktop_runtime_lock_validate"',
  ]) {
    assert.match(scriptsBuild, new RegExp(target.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&")));
  }

  assert.match(scriptsBuild, /srcs = \["\/\/tools\/bazel:run_workspace_task\.sh"\]/);
  assert.match(scriptsBuild, /args = \["core", "node", "--test", "scripts\/buildkite_pipeline_contract\.test\.cjs"\]/);
  assert.match(scriptsBuild, /args = \["core", "pnpm", "provider-auth:validate"\]/);
  assert.match(scriptsBuild, /args = \["core", "pnpm", "desktop:check:versions"\]/);
  assert.match(scriptsBuild, /args = \["core", "pnpm", "desktop:runtime:lock:check-matrix"\]/);
  assert.match(scriptsBuild, /args = \["core", "node", "scripts\/runtime_lock_validate\.cjs", "--profile", "parity"\]/);
});
