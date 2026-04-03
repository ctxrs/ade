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
  assert.match(
    text,
    /avf_restore_smoke_mode:[\s\S]*type: choice[\s\S]*options:[\s\S]*-\s+skip[\s\S]*-\s+required/
  );
  assert.equal((text.match(/type:\s+choice/g) || []).length, 1);
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

test("release supabase mac runtime-install smoke uses the bundled ctx-daemon path", () => {
  const text = workflowText("release-supabase.yml");
  assert.match(
    text,
    /name:\s+Pre-notary runtime-install smoke[\s\S]*bundled_daemon="\$app_path\/Contents\/MacOS\/ctx-daemon"[\s\S]*scripts\/release_runtime_install_smoke\.sh[\s\S]*--daemon-bin "\$daemon_bin"/s,
  );
});

test("release supabase preserves AVF helper virtualization entitlements in the final shipped mac app", () => {
  const text = workflowText("release-supabase.yml");
  assert.match(
    text,
    /helper_entitlements="core\/apps\/desktop\/src-tauri\/ctx-avf-linux-helper\.entitlements"/,
  );
  assert.match(
    text,
    /app_entitlements="core\/apps\/desktop\/src-tauri\/ctx\.entitlements"/,
  );
  assert.match(text, /assert_virtualization_entitlement\(\) \{/);
  assert.match(text, /assert_app_automation_entitlement\(\) \{/);
  assert.match(
    text,
    /codesign_with_retry --force --sign "\$identity" --options runtime --entitlements "\$helper_entitlements" --timestamp "\$helper_bin"/,
  );
  assert.match(
    text,
    /codesign_with_retry --force --sign "\$identity" --options runtime --entitlements "\$app_entitlements" --timestamp "\$app_path"/,
  );
  assert.doesNotMatch(
    text,
    /codesign_with_retry --force --deep --sign "\$identity" --options runtime --timestamp "\$app_path"/,
  );
});

test("release supabase validates release-stage replay sources against prereq artifacts on the same SHA", () => {
  const text = workflowText("release-supabase.yml");
  assert.match(text, /name:\s+resolve release-stage source run/);
  assert.match(text, /REQUESTED_RUN_ID:\s*\$\{\{\s*needs\.resolve-build-matrix\.outputs\.release_stage_source_run_id\s*\}\}/);
  assert.match(text, /TARGET_SHA:\s*\$\{\{\s*github\.sha\s*\}\}/);
  assert.match(text, /const requiredArtifacts = new Set\(\[[\s\S]*"web-dist"[\s\S]*"desktop-bundles-linux-merged"[\s\S]*daemon-sidecars-\$\{platform\}/s);
  assert.match(text, /Replay mode only supports prereq reuse from the same source commit\./);
  assert.match(text, /Use a completed release run for the same SHA that produced prereq artifacts, or rerun Release \(Supabase Storage\) without release_stage_source_run_id\./);
  assert.match(text, /RELEASE_STAGE_SOURCE_RUN_ID:\s*\$\{\{\s*needs\.resolve-release-stage-source\.outputs\.run_id\s*\}\}/);
});

test("release supabase requires same-SHA dependency bundles", () => {
  const text = workflowText("release-supabase.yml");
  assert.match(text, /resolve dependency bundles source run/);
  assert.match(
    text,
    /dependency bundle run \$\{requestedRunIdRaw\} targets \$\{run\.head_sha \|\| "unknown"\}, but release target is \$\{targetSha\}\. Release requires same-SHA dependency bundles\./,
  );
  assert.match(
    text,
    /no successful dependency-bundles run with required artifacts was found for target SHA \$\{targetSha\} on branch \$\{defaultBranch\}; trigger dependency-bundles for this SHA or set dependency_bundle_source_run_id to a same-SHA run explicitly\./,
  );
  assert.doesNotMatch(text, /using compatible run .* relying on downstream runtime-lock validation/);
});

test("release supabase publish restore accepts both src-tauri and core target bundle roots", () => {
  const text = workflowText("release-supabase.yml");
  assert.match(
    text,
    /name:\s+Upload release payload[\s\S]*core\/apps\/desktop\/src-tauri\/target\/release\/bundle[\s\S]*core\/target\/release\/bundle/s,
  );
  assert.match(
    text,
    /name:\s+Restore staged release payload[\s\S]*core\/apps\/desktop\/src-tauri\/target\/release\/bundle[\s\S]*apps\/desktop\/src-tauri\/target\/release\/bundle[\s\S]*core\/target\/release\/bundle[\s\S]*target\/release\/bundle/s,
  );
  assert.match(
    text,
    /name:\s+Restore staged release payload \(mac variant\)[\s\S]*core\/apps\/desktop\/src-tauri\/target\/release\/bundle[\s\S]*apps\/desktop\/src-tauri\/target\/release\/bundle[\s\S]*core\/target\/release\/bundle[\s\S]*target\/release\/bundle/s,
  );
});

test("release supabase linux release payload upload includes the current core target bundle root", () => {
  const text = workflowText("release-supabase.yml");
  assert.match(
    text,
    /if \[\[ ! -s "\$\{appimage\}\.sig" \]\]; then[\s\S]*name:\s+Upload release payload[\s\S]*core\/apps\/desktop\/src-tauri\/target\/release\/bundle[\s\S]*core\/target\/release\/bundle[\s\S]*core\/target\/release\/ctx/s,
  );
});

test("linux tauri release container reuses staged bundles instead of rematerializing them", () => {
  const text = fs.readFileSync(path.join(repoRoot, "scripts", "release_linux_tauri_bundle_in_container.sh"), "utf8");
  assert.match(text, /CTX_DESKTOP_SYNC_BUNDLES=0/);
  assert.match(text, /CTX_BUNDLE_REMOTE_DAEMONS=0/);
  assert.match(text, /pnpm -C core\/apps\/desktop run build -- --bundles appimage/);
});

test("non-linux release-stage tauri builds reuse staged bundles instead of rematerializing them", () => {
  const text = workflowText("release-supabase.yml");
  assert.match(
    text,
    /if \[\[ "\$RELEASE_PLATFORM" == macos-\* \]\]; then[\s\S]*CTX_DESKTOP_SYNC_BUNDLES=0 CTX_BUNDLE_REMOTE_DAEMONS=0 pnpm -C core\/apps\/desktop run build -- --bundles app[\s\S]*else[\s\S]*CTX_DESKTOP_SYNC_BUNDLES=0 CTX_BUNDLE_REMOTE_DAEMONS=0 pnpm -C core\/apps\/desktop run build/s,
  );
});

test("mac updater smoke reuses staged bundles instead of rematerializing them", () => {
  const text = workflowText("release-supabase.yml");
  assert.match(
    text,
    /name:\s+Updater desktop apply smoke \(macOS tier1\)[\s\S]*CTX_DESKTOP_APP_PATH:\s*\$\{\{\s*runner\.temp\s*\}\}\/ctx-e2e-cargo\/debug\/bundle\/macos[\s\S]*CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD:\s*"1"[\s\S]*CTX_DESKTOP_SYNC_BUNDLES=0 CTX_BUNDLE_REMOTE_DAEMONS=0 CARGO_TARGET_DIR="\$\{CTX_E2E_CARGO_TARGET_DIR\}" pnpm -C core\/apps\/desktop run build -- --debug --bundles app -- --features automation/s,
  );
  assert.match(
    text,
    /name:\s+Updater desktop apply smoke \(macOS variant\)[\s\S]*CTX_DESKTOP_APP_PATH:\s*\$\{\{\s*runner\.temp\s*\}\}\/ctx-e2e-cargo\/debug\/bundle\/macos[\s\S]*CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD:\s*"1"[\s\S]*CTX_DESKTOP_SYNC_BUNDLES=0 CTX_BUNDLE_REMOTE_DAEMONS=0 CARGO_TARGET_DIR="\$\{CTX_E2E_CARGO_TARGET_DIR\}" pnpm -C core\/apps\/desktop run build -- --debug --bundles app -- --features automation/s,
  );
});

test("release supabase publishes the AVF Linux guest runtime before the shipped-app first-run gate", () => {
  const text = workflowText("release-supabase.yml");
  assert.match(text, /- name:\s+Rust toolchain \(AVF runtime publish\)/);
  assert.match(
    text,
    /- name:\s+Install AVF runtime publish toolchain \(macOS arm64\)[\s\S]*command -v zig[\s\S]*brew install zig[\s\S]*command -v qemu-img[\s\S]*brew install qemu[\s\S]*command -v cargo-zigbuild[\s\S]*cargo install cargo-zigbuild --locked/s,
  );
  assert.match(
    text,
    /- name:\s+Publish AVF Linux guest runtime \(macOS arm64\)[\s\S]*runtime_dir="\$RUNNER_TEMP\/ctx-avf-linux-guest-runtime"[\s\S]*prepare_avf_linux_guest_runtime\.sh[\s\S]*avf_runtime_lock_freshness\.cjs[\s\S]*--allow-managed-runtime[\s\S]*avf_runtime_publish_supabase\.sh[\s\S]*--publish/s,
  );
  assert.match(
    text,
    /- name:\s+Probe published AVF Linux guest runtime URLs \(macOS arm64\)[\s\S]*runtime_urls_file="\$RUNNER_TEMP\/ctx-avf-runtime-urls\.txt"[\s\S]*runtime_lock\.v2\.json[\s\S]*rootfs\.raw\.zst[\s\S]*while IFS= read -r url; do[\s\S]*curl -fsSIL --retry 3 --retry-delay 1 "\$url"/s,
  );
  assert.doesNotMatch(text, /mapfile -t runtime_urls/);
});

test("release supabase validates managed ctx-harness freshness on Linux release lanes", () => {
  const text = workflowText("release-supabase.yml");
  assert.match(
    text,
    /- name:\s+Ensure Docker daemon is available \(ctx-harness freshness\)[\s\S]*sudo systemctl start docker \|\| sudo service docker start[\s\S]*docker version/s,
  );
  assert.match(
    text,
    /- name:\s+Setup Buildx \(ctx-harness freshness\)[\s\S]*docker\/setup-buildx-action@v3[\s\S]*driver:\s*docker-container/s,
  );
  assert.match(
    text,
    /- name:\s+Validate managed ctx-harness image freshness \(linux\)[\s\S]*ctx_harness_publish_supabase\.sh[\s\S]*--lock core\/apps\/desktop\/src-tauri\/bundles\/runtime_lock\.v2\.json[\s\S]*--check-only/s,
  );
  assert.match(
    text,
    /- name:\s+Probe published ctx-harness image URL \(linux\)[\s\S]*entry\.kind === "image"[\s\S]*entry\.id === "ctx-harness"[\s\S]*curl -fsSIL --retry 3 --retry-delay 1 "\$url"/s,
  );
});

test("release supabase runs a shipped-app first-run workspace gate on the Mac mini", () => {
  const text = workflowText("release-supabase.yml");
  const block = text.match(/mac-first-run-workspace-create:[\s\S]*?\n  verify-manifest:/)?.[0] || "";
  const runBlock = block.match(
    /- name:\s+Real first-run workspace create smoke \(macOS shipped app\)[\s\S]*?(?=\n\s*- name:|\n\s*verify-manifest:|$)/,
  )?.[0] || "";
  assert.match(text, /mac-first-run-workspace-create:/);
  assert.match(
    block,
    /mac-first-run-workspace-create:[\s\S]*runs-on:[\s\S]*group:\s*ctx-avf[\s\S]*labels:\s*ctx-avf-mac-mini/s,
  );
  assert.match(
    block,
    /name:\s+Install shipped macOS app \(first run\)[\s\S]*find "\$stage_root" -type f -name '\*\.dmg'[\s\S]*ditto "\$app_src" "\$dest_app"/s,
  );
  assert.match(
    block,
    /name:\s+Prepare current-source AVF runtime \(first run\)[\s\S]*prepare_avf_linux_guest_runtime\.sh[\s\S]*--output-dir "\$RUNNER_TEMP\/ctx-avf-linux-guest-runtime"/s,
  );
  assert.match(
    block,
    /name:\s+Validate CN_API_KEY secret \(mac first run\)[\s\S]*missing required GitHub secret CN_API_KEY for shipped-app macOS first-run smoke/s,
  );
  assert.match(
    block,
    /name:\s+Verify bundled AVF runtime freshness \(first run\)[\s\S]*CTX_FIRST_RUN_BUNDLE_DIR[\s\S]*node core\/scripts\/avf_runtime_lock_freshness\.cjs[\s\S]*--bundle-dir "\$\{CTX_FIRST_RUN_BUNDLE_DIR\}"[\s\S]*--allow-managed-runtime[\s\S]*--runtime-dir "\$RUNNER_TEMP\/ctx-avf-linux-guest-runtime"/s,
  );
  assert.match(
    block,
    /name:\s+Wipe local ctx state \(first run\)[\s\S]*pkill -f 'ctx-cnb-cli' \|\| true[\s\S]*pkill -f 'ctx-tdrv-cli' \|\| true[\s\S]*pkill -f 'WebKitWebDriver' \|\| true[\s\S]*pkill -f '\/Contents\/MacOS\/ctx\$' \|\| true[\s\S]*pkill -f '\/Contents\/Resources\/bin\/ctx-daemon' \|\| true[\s\S]*pkill -f '\/Contents\/Resources\/bin\/ctx-avf-linux-helper' \|\| true[\s\S]*pkill -f '\/Contents\/Resources\/bin\/ctx-mcp' \|\| true/s,
  );
  assert.match(
    runBlock,
    /name:\s+Real first-run workspace create smoke \(macOS shipped app\)[\s\S]*CTX_AUTOMATION_SHIPPED_APP:\s*"1"[\s\S]*CTX_AUTOMATION_SKIP_DESKTOP_PREP_RELEASE:\s*"1"[\s\S]*CTX_AUTOMATION_SKIP_APP_BUILD:\s*"1"[\s\S]*CTX_AUTOMATION_CN_SHARED_BACKEND:\s*"0"[\s\S]*CTX_AUTOMATION_FIRST_RUN_CONFIRM_DELAY_MS:\s*"10000"[\s\S]*CTX_AUTOMATION_SCENARIOS:\s*local-avf-first-run[\s\S]*CTX_AUTOMATION_FIRST_RUN_REPORT_PATH:\s*\$\{\{\s*runner\.temp\s*\}\}\/ctx-first-run-local-sandbox-report\.json[\s\S]*CTX_AUTOMATION_CN_BACKEND_LOG:\s*\$\{\{\s*runner\.temp\s*\}\}\/ctx-cn-test-runner-backend\.log[\s\S]*CTX_AUTOMATION_CN_DRIVER_LOG:\s*\$\{\{\s*runner\.temp\s*\}\}\/ctx-cn-tauri-driver\.log[\s\S]*CN_API_KEY:\s*\$\{\{\s*secrets\.CN_API_KEY\s*\}\}[\s\S]*CTX_DESKTOP_APP_PATH="\$APP_PATH"[\s\S]*pnpm -C core\/apps\/desktop run test:automation:first-run-local-sandbox/s,
  );
});

test("verify-manifest waits for the shipped-app first-run Mac mini gate", () => {
  const text = workflowText("release-supabase.yml");
  assert.match(
    text,
    /verify-manifest:[\s\S]*needs:[\s\S]*- mac-first-run-workspace-create[\s\S]*- publish-release[\s\S]*- resolve-build-matrix/s,
  );
  assert.match(
    text,
    /needs\.mac-first-run-workspace-create\.result == 'success' \|\| needs\.mac-first-run-workspace-create\.result == 'skipped'/,
  );
});
