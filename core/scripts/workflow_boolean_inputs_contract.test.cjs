const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const repoRoot = path.resolve(__dirname, "..", "..");

const workflowText = (name) =>
  fs.readFileSync(path.join(repoRoot, ".github", "workflows", name), "utf8");

test("desktop break matrix manual uses a typed boolean include_remote input", () => {
  const text = workflowText("desktop-break-matrix-manual.yml");
  assert.match(text, /include_remote:[\s\S]*type: boolean/);
  assert.match(text, /INPUT_INCLUDE_REMOTE:\s*\$\{\{\s*inputs\.include_remote && '1' \|\| '0'\s*\}\}/);
});

test("updater janitor uses a typed boolean dry_run input", () => {
  const text = workflowText("updater-e2e-janitor.yml");
  assert.match(text, /dry_run:[\s\S]*type: boolean/);
  assert.ok(!text.includes("inputs.dry_run == 'true'"));
});

test("publish claude-crp workflow uses a typed boolean publish input", () => {
  const text = workflowText("publish-claude-crp-provider-deps.yml");
  assert.match(text, /publish_to_supabase:[\s\S]*type: boolean/);
  assert.match(text, /if:\s*\$\{\{\s*github\.event_name == 'workflow_dispatch' && inputs\.publish_to_supabase\s*\}\}/);
});

test("release preflight uses a typed boolean override input", () => {
  const text = workflowText("release-preflight.yml");
  assert.match(text, /linux_arm_critical_override:[\s\S]*type: boolean/);
  assert.ok(!text.includes("type: choice"));
  assert.match(text, /1\|true\|yes\|on/);
});

test("release preflight provisions the native sandbox CLI before the linux-arm critical lane", () => {
  const text = workflowText("release-preflight.yml");
  assert.match(
    text,
    /name:\s+Install Linux sandbox runtime CLI[\s\S]*install_linux_sandbox_runtime_cli\.sh --require-reachable[\s\S]*ctx-rootful-nerdctl[\s\S]*CTX_HARNESS_SANDBOX_CLI_PATH[\s\S]*CONTAINERD_ADDRESS=\/run\/containerd\/containerd\.sock[\s\S]*CONTAINERD_NAMESPACE=default[\s\S]*name:\s+Run linux-arm critical provider lane/s,
  );
});

test("release supabase uses typed updater drill booleans", () => {
  const text = workflowText("release-supabase.yml");
  assert.match(text, /run_updater_e2e_drill:[\s\S]*type: boolean/);
  assert.match(text, /updater_e2e_expect_version_change:[\s\S]*type: boolean/);
  assert.ok(!text.includes("inputs.run_updater_e2e_drill == 'true'"));
  assert.match(text, /CTX_UPDATER_E2E_EXPECT_VERSION_CHANGE:\s*\$\{\{\s*github\.event_name == 'workflow_dispatch'/);
});

test("release supabase uses repo-configurable Linux runner labels", () => {
  const text = workflowText("release-supabase.yml");
  assert.match(text, /runs-on:\s*\$\{\{\s*vars\.RELEASE_RUNNER_LINUX_X64 \|\| 'linux-x64-8core'\s*\}\}/);
  assert.match(text, /runs-on:\s*\$\{\{\s*vars\.RELEASE_RUNNER_LINUX_ARM64 \|\| 'linux-arm-8core'\s*\}\}/);
  assert.match(text, /RELEASE_RUNNER_LINUX_X64:\s*\$\{\{\s*vars\.RELEASE_RUNNER_LINUX_X64 \|\| 'linux-x64-8core'\s*\}\}/);
  assert.match(text, /RELEASE_RUNNER_LINUX_ARM64:\s*\$\{\{\s*vars\.RELEASE_RUNNER_LINUX_ARM64 \|\| 'linux-arm-8core'\s*\}\}/);
  assert.match(text, /const linuxX64Runner = String\(process\.env\.RELEASE_RUNNER_LINUX_X64 \|\| ""\)\.trim\(\);/);
  assert.match(text, /const linuxArm64Runner = String\(process\.env\.RELEASE_RUNNER_LINUX_ARM64 \|\| ""\)\.trim\(\);/);
});

test("linux bundle workflows preserve AVF runtime targets instead of rewriting them to linux", () => {
  const dependencyBundlesText = workflowText("dependency-bundles.yml");
  const releaseSupabaseText = workflowText("release-supabase.yml");

  for (const text of [dependencyBundlesText, releaseSupabaseText]) {
    assert.doesNotMatch(text, /lock\.required\.targets\.runtime = \["linux\/x86_64"\];/);
    assert.doesNotMatch(text, /lock\.required\.targets\.runtime = \["linux\/aarch64"\];/);
  }

  assert.match(dependencyBundlesText, /lock\.required\.targets\.image = \["linux\/x86_64"\];/);
  assert.match(dependencyBundlesText, /lock\.required\.targets\.image = \["linux\/aarch64"\];/);
  assert.match(releaseSupabaseText, /lock\.required\.targets\.image = \["linux\/x86_64"\];/);
  assert.match(releaseSupabaseText, /lock\.required\.targets\.image = \["linux\/aarch64"\];/);
});

test("release supabase release-stage jobs consume the merged desktop bundle artifact", () => {
  const text = workflowText("release-supabase.yml");
  assert.match(text, /name:\s+Download merged desktop bundle resources \(current run\)[\s\S]*name:\s+desktop-bundles-linux-merged/s);
  assert.match(text, /name:\s+Download merged desktop bundle resources \(source run\)[\s\S]*name:\s+desktop-bundles-linux-merged[\s\S]*run-id:\s+\$\{\{\s*env\.RELEASE_STAGE_SOURCE_RUN_ID\s*\}\}/s);
  assert.match(text, /name:\s+Stage merged desktop bundle resources[\s\S]*src_dir="\$RUNNER_TEMP\/desktop-bundles-linux-merged"[\s\S]*dest_dir="core\/apps\/desktop\/src-tauri\/bundles"[\s\S]*"\$dest_dir\/daemons"[\s\S]*"\$dest_dir\/runtime_manifest\.effective\.json"[\s\S]*test -f "\$dest_dir\/manifest\.json"[\s\S]*test -f "\$dest_dir\/runtime_lock\.v1\.json"[\s\S]*test -f "\$dest_dir\/runtime_lock\.v2\.json"/s);
});

test("release supabase prep desktop release resources uses an absolute cargo target dir", () => {
  const text = workflowText("release-supabase.yml");
  assert.match(text, /CTX_DESKTOP_SYNC_BUNDLES=0 CARGO_TARGET_DIR="\$PWD\/core\/target" node core\/scripts\/desktop_sync_resources\.cjs --profile release/);
});

test("linux tauri release container reuses staged bundles instead of rematerializing them", () => {
  const text = fs.readFileSync(path.join(repoRoot, "scripts", "release_linux_tauri_bundle_in_container.sh"), "utf8");
  assert.match(text, /CTX_DESKTOP_SYNC_BUNDLES=0/);
  assert.match(text, /CTX_BUNDLE_REMOTE_DAEMONS=0/);
  assert.match(text, /pnpm -C core\/apps\/desktop run build -- --bundles appimage/);
});
