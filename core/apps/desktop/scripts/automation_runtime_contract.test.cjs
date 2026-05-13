const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.resolve(__dirname, "..");
const REPO_ROOT = path.resolve(ROOT, "..", "..", "..");
const TAURI_CARGO = path.join(ROOT, "src-tauri", "Cargo.toml");
const TAURI_CONF = path.join(ROOT, "src-tauri", "tauri.conf.json");
const APP_ENTITLEMENTS = path.join(ROOT, "src-tauri", "ctx.entitlements");
const TAURI_MAIN = path.join(ROOT, "src-tauri", "src", "main.rs");
const PACKAGE_JSON = path.join(ROOT, "package.json");
const WDIO_CONF = path.join(ROOT, "automation", "wdio.conf.cjs");
const WORKSPACE_WIZARD_SPEC = path.join(ROOT, "automation", "specs", "workspace-wizard.spec.cjs");
const WORKSPACE_WIZARD_FLOW_HELPER = path.join(ROOT, "automation", "specs", "helpers", "workspace_wizard_flow.cjs");
const DAEMON_RECONNECT_SPEC = path.join(ROOT, "automation", "specs", "daemon-reconnect.spec.cjs");
const RECENT_WORKSPACE_SPEC = path.join(ROOT, "automation", "specs", "recent-workspace-open.spec.cjs");
const LINUX_SANDBOX_LOCAL = path.join(ROOT, "src-tauri", "src", "linux_sandbox", "local.rs");
const DESKTOP_LOCAL_DAEMON = path.join(ROOT, "src-tauri", "src", "desktop_local_daemon.rs");
const REMOTE_REAL_CI_WRAPPER = path.join(ROOT, "scripts", "test_remote_real_ci.sh");
const REMOTE_DOCKER_WRAPPER = path.join(ROOT, "scripts", "test_remote_docker_contracts.sh");
const LINUX_BUNDLED_LAUNCH_SMOKE = path.join(ROOT, "scripts", "linux_bundled_launch_smoke.mjs");
const DESKTOP_SMOKE_WRAPPER = path.join(REPO_ROOT, "scripts", "desktop_smoke_with_infisical.sh");
const LINUX_LOCAL_TRUTH_WRAPPER = path.join(REPO_ROOT, "scripts", "tests", "linux_local_install_sandbox_release_truth.sh");
const UPDATER_LINUX_PROOF_WRAPPER = path.join(REPO_ROOT, "scripts", "tests", "updater_linux_release_truth.sh");
const MAC_REMOTE_TRUTH_WRAPPER = path.join(REPO_ROOT, "scripts", "tests", "macos_remote_ubuntu_sandbox_release_truth.sh");
const UPDATER_REMOTE_WRAPPER = path.join(REPO_ROOT, "scripts", "tests", "updater_remote_daemon_e2e.sh");
const AUTOMATION_SPECS_DIR = path.join(ROOT, "automation", "specs");

const collectCjsFiles = (dir) => {
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  return entries.flatMap((entry) => {
    const entryPath = path.join(dir, entry.name);
    if (entry.isDirectory()) return collectCjsFiles(entryPath);
    return entry.name.endsWith(".cjs") ? [entryPath] : [];
  });
};

test("production desktop build keeps automation runtime available", () => {
  const cargo = fs.readFileSync(TAURI_CARGO, "utf8");
  assert.match(cargo, /^tauri-plugin-automation\s*=\s*"0\.1\.1"$/m);
  assert.doesNotMatch(cargo, /^tauri-plugin-automation\s*=.*optional\s*=\s*true/m);
  assert.match(cargo, /^automation\s*=\s*\[\]$/m);

  const main = fs.readFileSync(TAURI_MAIN, "utf8");
  assert.match(main, /use tauri_plugin_automation::init as automation_init;/);
  assert.match(main, /builder = builder\.plugin\(automation_init\(\)\);/);
  assert.match(
    main,
    /fn desktop_automation_runtime_enabled\(\) -> bool[\s\S]*TAURI_WEBVIEW_AUTOMATION[\s\S]*AUTOMATION_LIBRARY_PATH/,
  );
  assert.match(
    main,
    /#\[cfg\(not\(feature = "automation"\)\)\]\s*\{\s*if !desktop_automation_runtime_enabled\(\) \{\s*builder = builder\.plugin\(tauri_plugin_single_instance::init/s,
  );
  assert.doesNotMatch(
    main,
    /#\[cfg\(feature = "automation"\)\]\s*\{\s*builder = builder\.plugin\(automation_init\(\)\);/s,
  );
});

test("production desktop build disables macOS library validation for shipped-app automation", () => {
  const tauriConf = JSON.parse(fs.readFileSync(TAURI_CONF, "utf8"));
  assert.equal(tauriConf.bundle?.macOS?.entitlements, "ctx.entitlements");

  const entitlements = fs.readFileSync(APP_ENTITLEMENTS, "utf8");
  assert.match(entitlements, /<key>com\.apple\.security\.cs\.disable-library-validation<\/key>/);
  assert.match(entitlements, /<true\/>/);
});

test("local Linux sandbox preparation restarts the invoking window daemon scope", () => {
  const source = fs.readFileSync(LINUX_SANDBOX_LOCAL, "utf8");
  assert.match(source, /pub\(crate\) async fn desktop_ensure_local_linux_sandbox_ready\(\s*app: tauri::AppHandle,\s*window: tauri::Window,/s);
  assert.match(source, /let scope = window\.label\(\)\.to_string\(\);/);
  assert.match(source, /restart_local_with_spawn_for_scope\(&scope, manager,/);
  assert.doesNotMatch(source, /restart_local_with_spawn\(manager,/);
});

test("local daemon restart shares the connect gate", () => {
  const source = fs.readFileSync(DESKTOP_LOCAL_DAEMON, "utf8");
  const functionStart = source.indexOf("pub(super) fn restart_local_with_spawn_for_scope");
  const gateIndex = source.indexOf("let _guard = lock_local_connect_gate()?;", functionStart);
  const disconnectIndex = source.indexOf("disconnect_owned_local_daemons_for_restart", functionStart);
  assert.ok(functionStart > 0, "restart helper should stay present");
  assert.ok(gateIndex > functionStart, "restart helper should acquire the local connect gate");
  assert.ok(disconnectIndex > gateIndex, "restart helper should acquire the gate before disconnecting daemons");
  assert.match(source, /fn lock_local_connect_gate\(\) -> Result<std::sync::MutexGuard<'static, \(\)>>/);
});

test("desktop automation asserts browser-scoped daemon connection info", () => {
  for (const file of [
    WORKSPACE_WIZARD_SPEC,
    WORKSPACE_WIZARD_FLOW_HELPER,
    DAEMON_RECONNECT_SPEC,
    RECENT_WORKSPACE_SPEC,
  ]) {
    const source = fs.readFileSync(file, "utf8");
    assert.match(source, /browser_query_secret/);
    assert.doesNotMatch(source, /base_url\+token|base_url \+ token/);
    assert.doesNotMatch(source, /info\.base_url && info\.token/);
    assert.doesNotMatch(source, /refreshed\.base_url && refreshed\.token/);
    assert.doesNotMatch(source, /connection\.token/);
  }
});

test("first-run local sandbox script defaults to isolated macOS CN backend", () => {
  const pkg = JSON.parse(fs.readFileSync(PACKAGE_JSON, "utf8"));
  assert.match(
    String(pkg.scripts["test:automation:first-run-local-sandbox"] || ""),
    /CTX_AUTOMATION_CN_SHARED_BACKEND=\$\{CTX_AUTOMATION_CN_SHARED_BACKEND:-0\}/,
  );
});

test("updater native smoke script opts out of container scenario defaults", () => {
  const pkg = JSON.parse(fs.readFileSync(PACKAGE_JSON, "utf8"));
  const script = String(pkg.scripts["test:automation:updater-native-smoke"] || "");
  assert.match(
    script,
    /CTX_AUTOMATION_SCENARIOS=\$\{CTX_AUTOMATION_SCENARIOS:-updater-native-smoke\}/,
  );
  assert.match(
    script,
    /CTX_AUTOMATION_SKIP_DESKTOP_PREP_RELEASE=\$\{CTX_AUTOMATION_SKIP_DESKTOP_PREP_RELEASE:-1\}/,
  );
  assert.match(
    script,
    /\.\.\/\.\.\/\.\.\/scripts\/desktop_smoke_with_infisical\.sh -- --spec automation\/specs\/updater-native-smoke\.spec\.cjs/,
  );
});

test("updater remote script runs through the shipped-app desktop smoke wrapper", () => {
  const pkg = JSON.parse(fs.readFileSync(PACKAGE_JSON, "utf8"));
  const script = String(pkg.scripts["test:automation:updater-remote"] || "");
  assert.match(
    script,
    /CTX_AUTOMATION_SCENARIOS=\$\{CTX_AUTOMATION_SCENARIOS:-updater-remote\}/,
  );
  assert.match(
    script,
    /CTX_AUTOMATION_SKIP_DESKTOP_PREP_RELEASE=\$\{CTX_AUTOMATION_SKIP_DESKTOP_PREP_RELEASE:-1\}/,
  );
  assert.match(
    script,
    /\.\.\/\.\.\/\.\.\/scripts\/desktop_smoke_with_infisical\.sh -- --spec automation\/specs\/updater-remote-daemon-e2e\.spec\.cjs/,
  );
});

test("updater remote wrapper uses shipped app without source-side provisioning", () => {
  const script = fs.readFileSync(UPDATER_REMOTE_WRAPPER, "utf8");
  assert.match(script, /updater_e2e_remote_matrix\.sh" run -- bash/);
  assert.match(script, /CTX_UPDATER_E2E_SSH_KEY_PATH\/CTX_AUTOMATION_REMOTE_SSH_KEY_PATH is required/);
  assert.match(script, /RELEASE_STORAGE_CHANNEL:-\$\{RELEASE_CHANNEL:-stable\}/);
  assert.match(script, /download_published_controller_app\(\)/);
  assert.match(script, /latest manifest missing linux-x64 AppImage artifact/);
  assert.match(script, /sha256sum -c -/);
  assert.match(script, /CTX_UPDATER_E2E_EXPECT_VERSION_CHANGE="\$\{CTX_UPDATER_E2E_EXPECT_VERSION_CHANGE:-1\}"/);
  assert.match(script, /CTX_AUTOMATION_SHIPPED_APP="\$\{CTX_AUTOMATION_SHIPPED_APP:-1\}"/);
  assert.match(script, /CTX_AUTOMATION_SKIP_APP_BUILD="\$\{CTX_AUTOMATION_SKIP_APP_BUILD:-1\}"/);
  assert.match(script, /CTX_AUTOMATION_SKIP_REMOTE_CTX_PROVISION="\$\{CTX_AUTOMATION_SKIP_REMOTE_CTX_PROVISION:-1\}"/);
  assert.match(script, /local attempt_dir="\$\{artifact_dir\}\/automation-attempt-\$\{attempt\}"/);
  assert.match(script, /local attempt_tmp_dir="\$\{attempt_dir\}\/tmp"/);
  assert.match(script, /CTX_AUTOMATION_TMPDIR="\$\{attempt_tmp_dir\}"/);
  assert.match(script, /CTX_AUTOMATION_SHIPPED_APP_DAEMON_DATA_DIR="\$\{attempt_dir\}\/controller-daemon-data"/);
  assert.match(script, /CTX_AUTOMATION_ALLOW_PREP_APP_PROCESS_SWEEP="\$\{CTX_AUTOMATION_ALLOW_PREP_APP_PROCESS_SWEEP:-1\}"/);
  assert.match(script, /CTX_AUTOMATION_ALLOW_STALE_HELPER_SWEEP="\$\{CTX_AUTOMATION_ALLOW_STALE_HELPER_SWEEP:-1\}"/);
  assert.match(script, /sweep_webkit_automation_helpers\(\) \{/);
  assert.match(script, /sweep_local_automation_daemons\(\) \{/);
  assert.match(script, /sweep_local_automation_processes/);
  assert.match(script, /ctx-desktop-e2e-app-daemon-/);
  assert.match(script, /controller-daemon-data/);
  assert.match(script, /unset CTX_BUNDLE_DIR/);
});

test("remote real CI wrapper retries startup-only WebDriver session failures", () => {
  const script = fs.readFileSync(REMOTE_REAL_CI_WRAPPER, "utf8");
  const wdio = fs.readFileSync(WDIO_CONF, "utf8");
  assert.match(script, /CTX_REMOTE_REAL_CI_AUTOMATION_ATTEMPTS/);
  assert.match(script, /is_retryable_wdio_session_start_failure\(\) \{/);
  assert.match(script, /UND_ERR_HEADERS_TIMEOUT/);
  assert.match(script, /hyper::Error\\?\(IncompleteMessage\\?\)/);
  assert.match(script, /Could not start a new session/);
  assert.match(script, /write_launch_diagnostics\(\) \{/);
  assert.match(script, /sweep_webkit_automation_helpers\(\) \{/);
  assert.match(script, /sweep_stale_xvfb_processes\(\) \{/);
  assert.match(script, /remote-workspace-desktop-launch-smoke/);
  assert.match(script, /sweep_local_automation_processes\(\) \{/);
  assert.match(script, /sweep_xvfb_processes_for_tmp_dir "\$\{attempt_tmp_dir\}"/);
  assert.match(script, /write_process_snapshot "\$\{attempt_dir\}\/processes-before-sweep\.log"/);
  assert.match(script, /write_process_snapshot "\$\{attempt_dir\}\/processes-after-automation-sweep\.log"/);
  assert.match(script, /write_launch_diagnostics "\$\{attempt_dir\}\/launch-env\.txt"/);
  assert.match(script, /attempt_daemon_data_dir="\$\{attempt_dir\}\/controller-daemon-data"/);
  assert.match(script, /CTX_AUTOMATION_ALLOW_PREP_APP_PROCESS_SWEEP="\$\{CTX_AUTOMATION_ALLOW_PREP_APP_PROCESS_SWEEP:-1\}"/);
  assert.match(script, /CTX_AUTOMATION_ALLOW_STALE_HELPER_SWEEP="\$\{CTX_AUTOMATION_ALLOW_STALE_HELPER_SWEEP:-1\}"/);
  assert.match(script, /CTX_AUTOMATION_SHIPPED_APP_DAEMON_DATA_DIR="\$\{attempt_daemon_data_dir\}"/);
  assert.match(script, /CTX_AUTOMATION_APP_LAUNCH_LOG="\$\{attempt_dir\}\/app-launch\.log"/);
  assert.match(script, /CTX_AUTOMATION_CN_DRIVER_LOG="\$\{attempt_dir\}\/tauri-driver\.log"/);
  assert.match(wdio, /TAURI_WEBVIEW_AUTOMATION: "true"/);
  assert.match(wdio, /"APPDIR"/);
  assert.match(wdio, /"APPIMAGE"/);
  assert.match(wdio, /"CTX_APPIMAGE_PATH"/);
  assert.match(wdio, /\.\.\.DESKTOP_APP_LAUNCH_ENV,/);
  assert.match(wdio, /WebDriver application path=/);
  assert.match(script, /HOME="\$\{attempt_home_dir\}"/);
  assert.match(script, /TMPDIR="\$\{attempt_tmp_dir\}"/);
  assert.match(script, /TMP="\$\{attempt_tmp_dir\}"/);
  assert.match(script, /TEMP="\$\{attempt_tmp_dir\}"/);
  assert.match(script, /XDG_RUNTIME_DIR="\$\{attempt_xdg_runtime_dir\}"/);
  assert.match(script, /XDG_CONFIG_HOME="\$\{attempt_xdg_dir\}\/config"/);
  assert.match(script, /XDG_CACHE_HOME="\$\{attempt_xdg_dir\}\/cache"/);
  assert.match(script, /XDG_DATA_HOME="\$\{attempt_xdg_dir\}\/data"/);
  assert.match(script, /attempt_corepack_home="\$\{attempt_xdg_dir\}\/corepack"/);
  assert.match(script, /COREPACK_HOME="\$\{attempt_corepack_home\}"/);
  assert.match(script, /COREPACK_ENABLE_DOWNLOAD_PROMPT=0/);
  assert.match(script, /core_package_manager\(\) \{/);
  assert.match(script, /prepare_attempt_corepack\(\) \{/);
  assert.match(script, /corepack prepare "\$\{package_manager\}" --activate/);
  assert.match(script, /sweep_webkit_automation_helpers/);
  assert.match(script, /\[\[ -f "\$\{report_path\}" \]\]/);
  assert.match(script, /wdio-attempt-\$\{attempt\}\.log/);
  assert.match(script, /retrying startup-only WebDriver session failure/);
  assert.match(script, /rm -f "\$\{report_path\}"/);
});

test("linux bundled launch smoke passes explicit tauri-driver native port", () => {
  const script = fs.readFileSync(LINUX_BUNDLED_LAUNCH_SMOKE, "utf8");

  assert.match(script, /--native-port/);
  assert.match(script, /TAURI_DRIVER_NATIVE_PORT: String\(nativePort\)/);
  assert.match(script, /failed to allocate distinct tauri-driver native port/);
  assert.match(script, /requireWorkspacePackage\("webdriverio"\)/);
  assert.doesNotMatch(script, /demo_ping_pong_playback/);
  assert.doesNotMatch(script, /node:sqlite/);
});

test("updater Linux proof targets storage channel for stable dry-run proofs", () => {
  const script = fs.readFileSync(UPDATER_LINUX_PROOF_WRAPPER, "utf8");
  assert.match(script, /TARGET_CHANNEL="\$\{CTX_UPDATER_LINUX_PROOF_TARGET_CHANNEL:-\$\{RELEASE_STORAGE_CHANNEL:-\$\{RELEASE_CHANNEL:-e2e\}\}\}"/);
  assert.match(
    script,
    /WDIO_CONNECTION_RETRY_TIMEOUT_MS="\$\{CTX_UPDATER_LINUX_PROOF_WDIO_CONNECTION_RETRY_TIMEOUT_MS:-300000\}"/,
  );
  assert.equal(
    script.match(/CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS="\$\{CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS:-\$\{WDIO_CONNECTION_RETRY_TIMEOUT_MS\}\}"/g)?.length,
    3,
  );
  assert.equal(
    script.match(/APPIMAGE_EXTRACT_AND_RUN="\$\{APPIMAGE_EXTRACT_AND_RUN:-1\}"/g)?.length,
    3,
  );
  assert.match(script, /extract_appimage_for_automation\(\) \{/);
  assert.match(script, /APPIMAGE_AUTOMATION_TARGETS_LOG="\$\{ARTIFACT_DIR\}\/appimage-automation-targets\.log"/);
  assert.match(script, /DAEMON_CLEANUP_LOG="\$\{ARTIFACT_DIR\}\/daemon-cleanup\.log"/);
  assert.match(script, /stop_proof_daemons\(\) \{/);
  assert.match(script, /daemon\.lock/);
  assert.equal(
    script.match(/^\s*stop_proof_daemons$/gm)?.length,
    3,
  );
  assert.match(script, /CTX_DESKTOP_APP_PATH="\$\{bootstrap_automation_app_path\}"/);
  assert.equal(
    script.match(/CTX_DESKTOP_APP_PATH="\$\{updated_automation_app_path\}"/g)?.length,
    2,
  );
  assert.equal(
    script.match(/CTX_APPIMAGE_PATH="\$\{app_path\}"/g)?.length,
    3,
  );
  assert.equal(
    script.match(/APPIMAGE="\$\{app_path\}"/g)?.length,
    3,
  );
  assert.match(script, /prepare_webkit_runtime\(\) \{/);
  assert.match(
    script,
    /bash -lc 'source "\$1"; release_prepare_webkit_browser' _ "\$\{ROOT\}\/scripts\/buildbuddy\/release_job_lib\.sh" \|\| return/,
  );
  assert.match(script, /buildkite-agent artifact upload "\$\{ARTIFACT_DIR\}\/\*\.json"/);
  assert.match(script, /buildkite-agent artifact upload "\$\{ARTIFACT_DIR\}\/volatile\/artifacts\/ctx-desktop-e2e\/\*\*\/\*\.log"/);
  assert.doesNotMatch(script, /buildkite-agent artifact upload "\$\{ARTIFACT_DIR\}\/\*\*\/\*"/);
});

test("desktop smoke cleanup unmounts AppImage FUSE mounts even when logs are preserved", () => {
  const script = fs.readFileSync(DESKTOP_SMOKE_WRAPPER, "utf8");
  const cleanupStart = script.indexOf("cleanup_automation_tmpdir()");
  const findmnt = script.indexOf("findmnt -rn -o TARGET", cleanupStart);
  const fusermount = script.indexOf("fusermount3 -u", cleanupStart);
  const keepTmp = script.indexOf("CTX_AUTOMATION_KEEP_TMPDIR", cleanupStart);
  assert.notEqual(cleanupStart, -1);
  assert.notEqual(findmnt, -1);
  assert.notEqual(fusermount, -1);
  assert.notEqual(keepTmp, -1);
  assert.ok(
    findmnt < keepTmp,
    "FUSE unmount must run before keep-tmpdir returns",
  );
});

test("mac WDIO automation targets the app executable and performs a real session readiness probe", () => {
  const wdio = fs.readFileSync(WDIO_CONF, "utf8");
  assert.match(wdio, /const resolveWdioApplicationPath = \(appPath\) => \{/);
  assert.match(wdio, /const resolveAutomationApplicationPath = \(appPath\) => \{\s+const appExecutablePath = resolveWdioApplicationPath\(appPath\);/);
  assert.match(wdio, /const WDIO_APPLICATION_PATH = resolveAutomationApplicationPath\(APP_PATH\);/);
  assert.match(
    wdio,
    /"tauri:options":\s*\{[\s\S]*application:\s*WDIO_APPLICATION_PATH/,
  );
  assert.match(
    wdio,
    /before:\s*async\s*\(_capabilities,\s*_specs,\s*browser\)\s*=>\s*\{[\s\S]*await browser\.getWindowHandle\(\);/s,
  );
});

test("Tauri route navigation helper does not depend on WebDriver page-load completion", () => {
  const wdio = fs.readFileSync(WDIO_CONF, "utf8");
  const tauriHelper = fs.readFileSync(path.join(ROOT, "automation", "specs", "helpers", "tauri.cjs"), "utf8");
  const updaterNativeSmoke = fs.readFileSync(
    path.join(ROOT, "automation", "specs", "updater-native-smoke.spec.cjs"),
    "utf8",
  );
  const updaterRemoteDaemon = fs.readFileSync(
    path.join(ROOT, "automation", "specs", "updater-remote-daemon-e2e.spec.cjs"),
    "utf8",
  );
  const workspaceWizard = fs.readFileSync(
    path.join(ROOT, "automation", "specs", "workspace-wizard.spec.cjs"),
    "utf8",
  );
  assert.doesNotMatch(wdio, /pageLoadStrategy:\s*"none"/);
  assert.doesNotMatch(wdio, /installTauriNavigationWait/);
  assert.match(wdio, /timeouts:\s*\{[\s\S]*pageLoad:\s*300000/);
  assert.match(tauriHelper, /const navigateToTauriUrl = async/);
  assert.match(tauriHelper, /window\.location\.assign\(href\)/);
  assert.doesNotMatch(tauriHelper, /browser\.url\(/);
  assert.match(tauriHelper, /matchSearch: options\.matchSearch !== false/);
  assert.match(tauriHelper, /route\.matchSearch === false \|\| window\.location\.search === route\.search/);
  assert.match(tauriHelper, /window\.location\.protocol === route\.protocol/);
  assert.match(tauriHelper, /window\.location\.search === route\.search/);
  assert.match(updaterNativeSmoke, /navigateToTauriUrl\("tauri:\/\/localhost\/workspaces"\)/);
  assert.match(updaterRemoteDaemon, /navigateToTauriUrl\("tauri:\/\/localhost\/workspaces"\)/);
  assert.match(workspaceWizard, /navigateToTauriUrl\(`tauri:\/\/localhost\/workspace-setup\?e2e=\$\{Date\.now\(\)\}`\)/);
  const rawTauriNavigations = collectCjsFiles(AUTOMATION_SPECS_DIR)
    .flatMap((filePath) => {
      const text = fs.readFileSync(filePath, "utf8");
      return /browser\.url\([^;\n]*tauri:\/\//.test(text) ? [path.relative(ROOT, filePath)] : [];
    });
  assert.deepEqual(rawTauriNavigations, []);
});

test("macOS automation launcher treats CrabNebula backend as fixed-port", () => {
  const wdio = fs.readFileSync(path.join(ROOT, "automation", "wdio.conf.cjs"), "utf8");
  assert.match(wdio, /const FIXED_MACOS_CN_BACKEND_PORT = 3000;/);
  assert.match(wdio, /activeTestBackendPort = FIXED_MACOS_CN_BACKEND_PORT;/);
  assert.match(
    wdio,
    /CrabNebula backend currently binds fixed port \$\{FIXED_MACOS_CN_BACKEND_PORT\} on macOS; ignoring requested CTX_AUTOMATION_CN_BACKEND_PORT=/,
  );
});

test("desktop smoke wrapper preserves CrabNebula backend and driver logs by default", () => {
  const wrapper = fs.readFileSync(DESKTOP_SMOKE_WRAPPER, "utf8");
  assert.match(wrapper, /DEFAULT_CN_BACKEND_LOG="\$\{AUTOMATION_TMPDIR\}\/crabnebula-backend\.log"/);
  assert.match(wrapper, /DEFAULT_CN_DRIVER_LOG="\$\{AUTOMATION_TMPDIR\}\/tauri-driver\.log"/);
  assert.match(wrapper, /CTX_AUTOMATION_CN_BACKEND_LOG="\$\{CTX_AUTOMATION_CN_BACKEND_LOG:-\$\{DEFAULT_CN_BACKEND_LOG\}\}"/);
  assert.match(wrapper, /CTX_AUTOMATION_CN_DRIVER_LOG="\$\{CTX_AUTOMATION_CN_DRIVER_LOG:-\$\{DEFAULT_CN_DRIVER_LOG\}\}"/);
  assert.match(wrapper, /AUTOMATION_FAILED=0/);
  assert.match(wrapper, /AUTOMATION_FAILED=1/);
  assert.match(wrapper, /\[desktop-smoke\] preserving failed automation tmpdir \$\{AUTOMATION_TMPDIR\}/);
  assert.match(wrapper, /\[desktop-smoke\] CrabNebula backend log: \$\{CTX_AUTOMATION_CN_BACKEND_LOG\}/);
  assert.match(wrapper, /\[desktop-smoke\] CrabNebula driver log: \$\{CTX_AUTOMATION_CN_DRIVER_LOG\}/);
});

test("desktop smoke wrapper unmounts Linux AppImage FUSE mounts before temp cleanup", () => {
  const wrapper = fs.readFileSync(DESKTOP_SMOKE_WRAPPER, "utf8");
  assert.match(wrapper, /command -v findmnt >\/dev\/null 2>&1/);
  assert.match(wrapper, /findmnt -rn -o TARGET 2>\/dev\/null \| sort -r/);
  assert.match(wrapper, /fusermount3 -u "\$\{mount_target\}"/);
  assert.match(wrapper, /umount -l "\$\{mount_target\}"/);
  assert.match(wrapper, /"\$\{mount_target\}" != "\$\{AUTOMATION_TMPDIR\}\/"\*/);
});

test("remote real CI wrapper forwards the shared cargo target dir into both WDIO lanes", () => {
  const wrapper = fs.readFileSync(REMOTE_REAL_CI_WRAPPER, "utf8");
  const forwardedTargetDir = /"CARGO_TARGET_DIR=\$\{ROOT\}\/core\/target"/g;
  assert.equal(wrapper.match(forwardedTargetDir)?.length, 2);
});

test("remote real CI wrapper can skip the synthetic bootstrap lane when run_host is disabled", () => {
  const wrapper = fs.readFileSync(REMOTE_REAL_CI_WRAPPER, "utf8");
  assert.match(wrapper, /RUN_HOST="\$\{CTX_REMOTE_CI_RUN_HOST:-1\}"/);
  assert.match(wrapper, /--run-host\)/);
  assert.match(
    wrapper,
    /if \[\[ "\$\{RUN_HOST\}" == "1" \]\]; then[\s\S]*test:automation:remote-bootstrap[\s\S]*else[\s\S]*disabled by run_host=0/s,
  );
});

test("remote real CI wrapper bridges fixture SSH config into the real desktop app", () => {
  const wrapper = fs.readFileSync(REMOTE_REAL_CI_WRAPPER, "utf8");
  assert.match(
    wrapper,
    /if \[\[ -z "\$\{CTX_DESKTOP_SSH_CONFIG_PATH:-\}" \]\]; then[\s\S]*CTX_AUTOMATION_REMOTE_CONTAINER_FIXTURE_SSH_CONFIG[\s\S]*CTX_AUTOMATION_REMOTE_FIXTURE_SSH_CONFIG[\s\S]*export CTX_DESKTOP_SSH_CONFIG_PATH=/,
  );
});

test("docker remote contract wrapper uses a Bash 3.2-compatible env presence check", () => {
  const wrapper = fs.readFileSync(REMOTE_DOCKER_WRAPPER, "utf8");
  assert.match(wrapper, /if \[\[ -z "\$\{CTX_AUTOMATION_REMOTE_ALLOW_SKIP\+x\}" \]\]; then/);
  assert.doesNotMatch(wrapper, /\[\[ ! -v CTX_AUTOMATION_REMOTE_ALLOW_SKIP \]\]/);
});

test("docker remote contract wrapper starts a local managed release fixture when no download base is provided", () => {
  const wrapper = fs.readFileSync(REMOTE_DOCKER_WRAPPER, "utf8");
  assert.match(wrapper, /export CARGO_TARGET_DIR="\$\{CARGO_TARGET_DIR:-\$\{ROOT\}\/core\/target\}"/);
  assert.match(wrapper, /CTX_AUTOMATION_REMOTE_USE_LOCAL_RELEASE_FIXTURE:-1/);
  assert.match(wrapper, /export CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD=1/);
  assert.match(wrapper, /pnpm -C "\$\{ROOT\}\/core" desktop:prep:release/);
  assert.match(
    wrapper,
    /CTX_DESKTOP_SYNC_BUNDLES=0[\s\S]*CTX_BUNDLE_REMOTE_DAEMONS=0[\s\S]*CARGO_TARGET_DIR="\$\{CARGO_TARGET_DIR\}"[\s\S]*pnpm -C "\$\{ROOT\}\/core\/apps\/desktop" run build -- --bundles app -- --features automation/s,
  );
  assert.match(wrapper, /export CTX_DESKTOP_APP_PATH="\$\{LOCAL_RELEASE_APP_PATH\}"/);
  assert.match(
    wrapper,
    /if \[\[ -n "\$\{CTX_DESKTOP_APP_PATH:-\}" && -z "\$\{CTX_AUTOMATION_SKIP_APP_BUILD\+x\}" \]\]; then[\s\S]*export CTX_AUTOMATION_SKIP_APP_BUILD=1/s,
  );
  assert.match(
    wrapper,
    /node "\$\{RELEASE_FIXTURE_SCRIPT\}" start --state-file "\$\{RELEASE_FIXTURE_STATE_FILE\}" --channel "\$\{CTX_DESKTOP_CHANNEL:-stable\}"/,
  );
  assert.match(
    wrapper,
    /export CTX_DESKTOP_ALLOW_INSECURE_LOCAL_UPDATER_FOR_REMOTE_BOOTSTRAP=1/,
  );
});

test("docker remote contract wrapper auto-hydrates CN_API_KEY from Infisical on macOS when available", () => {
  const wrapper = fs.readFileSync(REMOTE_DOCKER_WRAPPER, "utf8");
  assert.match(wrapper, /INFISICAL_CONFIG_FILE="\$\{CTX_AUTOMATION_INFISICAL_CONFIG_FILE:-\$\{ROOT\}\/core\/\.infisical\.json\}"/);
  assert.match(wrapper, /INFISICAL_REEXEC_MARKER="\$\{CTX_REMOTE_DOCKER_CONTRACTS_INFISICAL_REEXEC:-0\}"/);
  assert.match(wrapper, /can_run_with_infisical\(\) \{/);
  assert.match(wrapper, /maybe_reexec_with_infisical\(\) \{/);
  assert.match(
    wrapper,
    /exec infisical run --env "\$\{INFISICAL_ENV\}" --projectId "\$\{INFISICAL_PROJECT_ID\}" -- \\\s+env CTX_REMOTE_DOCKER_CONTRACTS_INFISICAL_REEXEC=1 "\$0" "\$@"/s,
  );
  assert.match(wrapper, /maybe_reexec_with_infisical "\$@"/);
});

test("docker remote contract wrapper defaults the fixture host mode to fresh-install", () => {
  const wrapper = fs.readFileSync(REMOTE_DOCKER_WRAPPER, "utf8");
  assert.match(
    wrapper,
    /export CTX_AUTOMATION_REMOTE_FIXTURE_HOST_MODE="\$\{CTX_AUTOMATION_REMOTE_FIXTURE_HOST_MODE:-fresh-install\}"/,
  );
});

test("docker remote contract wrapper repairs missing desktop automation toolchain before prep", () => {
  const wrapper = fs.readFileSync(REMOTE_DOCKER_WRAPPER, "utf8");
  assert.match(wrapper, /ensure_desktop_automation_deps\(\) \{/);
  assert.match(
    wrapper,
    /if \[\[ -d node_modules \]\] \\\s+&& pnpm -C apps\/web exec which vite >\/dev\/null 2>&1 \\\s+&& pnpm -C apps\/desktop exec which wdio >\/dev\/null 2>&1; then/s,
  );
  assert.match(wrapper, /\[remote-contracts\] repairing missing core\/apps\/web\/apps\/desktop automation toolchain/);
  assert.match(wrapper, /pnpm install --frozen-lockfile >\/dev\/null/);
  assert.match(wrapper, /pnpm -C apps\/web install --frozen-lockfile >\/dev\/null/);
  assert.match(wrapper, /pnpm -C apps\/desktop install --frozen-lockfile >\/dev\/null/);
  assert.match(
    wrapper,
    /if \[\[ "\$\(uname -s\)" == "Darwin" && -z "\$\{CN_API_KEY:-\}" \]\]; then[\s\S]*fi\n\nensure_desktop_automation_deps/s,
  );
});

test("docker remote contract wrapper pins volatile temp paths inside its artifact root", () => {
  const wrapper = fs.readFileSync(REMOTE_DOCKER_WRAPPER, "utf8");
  assert.match(
    wrapper,
    /if \[\[ -n "\$\{ARTIFACTS_DIR\}" \]\]; then[\s\S]*export CTX_VOLATILE_ARTIFACTS_DIR="\$\{CTX_VOLATILE_ARTIFACTS_DIR:-\$\{ARTIFACTS_DIR\}\}"[\s\S]*export CTX_VOLATILE_ROOT="\$\{CTX_VOLATILE_ROOT:-\$\{ARTIFACTS_DIR\}\/volatile\}"[\s\S]*else/s,
  );
  assert.match(wrapper, /mktemp -d \/tmp\/ctx-remote-contracts-volatile\.XXXXXX/);
  assert.match(
    wrapper,
    /export CTX_VOLATILE_TMPDIR="\$\{CTX_VOLATILE_TMPDIR:-\$\{CTX_VOLATILE_ROOT\}\/tmp\}"/,
  );
  assert.match(wrapper, /mkdir -p "\$\{CTX_VOLATILE_ROOT\}" "\$\{CTX_VOLATILE_TMPDIR\}"/);
  assert.match(
    wrapper,
    /export CTX_AUTOMATION_CN_BACKEND_STATE_DIR="\$\{CTX_AUTOMATION_CN_BACKEND_STATE_DIR:-\$\{CTX_VOLATILE_ROOT\}\/cn-backend\}"/,
  );
  assert.match(
    wrapper,
    /export CTX_AUTOMATION_CN_BACKEND_LOG="\$\{CTX_AUTOMATION_CN_BACKEND_LOG:-\$\{CTX_VOLATILE_ROOT\}\/crabnebula-backend\.log\}"/,
  );
  assert.match(
    wrapper,
    /export CTX_AUTOMATION_CN_DRIVER_LOG="\$\{CTX_AUTOMATION_CN_DRIVER_LOG:-\$\{CTX_VOLATILE_ROOT\}\/tauri-driver\.log\}"/,
  );
  assert.match(
    wrapper,
    /export CTX_AUTOMATION_CN_SHARED_BACKEND="\$\{CTX_AUTOMATION_CN_SHARED_BACKEND:-0\}"/,
  );
  assert.match(wrapper, /mkdir -p "\$\{CTX_AUTOMATION_CN_BACKEND_STATE_DIR\}"/);
  assert.match(wrapper, /pick_unused_local_port\(\) \{/);
  assert.match(wrapper, /start_remote_daemon_tunnel\(\) \{/);
  assert.match(
    wrapper,
    /export CTX_AUTOMATION_REMOTE_DIRECT_DAEMON_URL="http:\/\/127\.0\.0\.1:\$\{local_port\}"/,
  );
  assert.match(wrapper, /start_remote_daemon_tunnel/);
});

test("docker remote contract wrapper defaults to the real remote sandbox wizard lane", () => {
  const wrapper = fs.readFileSync(REMOTE_DOCKER_WRAPPER, "utf8");
  assert.match(wrapper, /RUN_HOST="\$\{CTX_REMOTE_CI_RUN_HOST:-0\}"/);
  assert.match(wrapper, /CMD=\("\$\{RUNNER_SCRIPT\}" "--run-container" "\$\{RUN_CONTAINER\}"\)/);
  assert.match(wrapper, /CMD\+=\("--run-host" "\$\{RUN_HOST\}"\)/);
});

test("linux local install truth wrapper validates the installed AppImage through the real workspace wizard lane", () => {
  const wrapper = fs.readFileSync(LINUX_LOCAL_TRUTH_WRAPPER, "utf8");
  assert.match(wrapper, /curl -fsSL '\$\{INSTALL_URL\}' \| sh/);
  assert.match(wrapper, /ctx\.AppImage/);
  assert.match(wrapper, /--appimage-extract/);
  assert.match(wrapper, /CTX_AUTOMATION_SHIPPED_APP=1/);
  assert.match(wrapper, /CTX_AUTOMATION_SHIPPED_APP_BUNDLES_DIR="\$\{bundle_dir\}"/);
  assert.match(wrapper, /workspace-wizard\.spec\.cjs/);
  assert.match(wrapper, /CTX_AUTOMATION_SCENARIOS="\$\{CTX_AUTOMATION_SCENARIOS:-local-codex-smoke\}"/);
  assert.match(wrapper, /runtime_bootstrap_missing/);
});

test("mac remote truth wrapper runs the real remote matrix and rejects docker-backed proof scopes", () => {
  const wrapper = fs.readFileSync(MAC_REMOTE_TRUTH_WRAPPER, "utf8");
  assert.match(wrapper, /CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS=1/);
  assert.match(wrapper, /CTX_AUTOMATION_CN_SHARED_BACKEND=0/);
  assert.match(wrapper, /REMOTE_MATRIX_SCRIPT="\$\{ROOT\}\/scripts\/updater_e2e_remote_matrix\.sh"/);
  assert.match(wrapper, /"\$\{REMOTE_MATRIX_SCRIPT\}" run --/);
  assert.match(wrapper, /REMOTE_REAL_SCRIPT="\$\{ROOT\}\/core\/apps\/desktop\/scripts\/test_remote_real_ci\.sh"/);
  assert.match(wrapper, /"\$\{REMOTE_REAL_SCRIPT\}"/);
  assert.match(wrapper, /expectedScope:\s*"remote_host"/);
  assert.match(wrapper, /expectedScope:\s*"remote_sandbox"/);
  assert.match(wrapper, /prepared docker fixture cannot satisfy release truth/);
});
