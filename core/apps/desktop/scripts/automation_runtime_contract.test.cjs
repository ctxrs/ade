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
const REMOTE_REAL_CI_WRAPPER = path.join(ROOT, "scripts", "test_remote_real_ci.sh");
const REMOTE_DOCKER_WRAPPER = path.join(ROOT, "scripts", "test_remote_docker_contracts.sh");
const DESKTOP_SMOKE_WRAPPER = path.join(REPO_ROOT, "scripts", "desktop_smoke_with_infisical.sh");
const LINUX_LOCAL_TRUTH_WRAPPER = path.join(REPO_ROOT, "scripts", "tests", "linux_local_install_sandbox_release_truth.sh");
const MAC_REMOTE_TRUTH_WRAPPER = path.join(REPO_ROOT, "scripts", "tests", "macos_remote_ubuntu_sandbox_release_truth.sh");

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
    /#\[cfg\(not\(feature = "automation"\)\)\]\s*\{\s*builder = builder\.plugin\(tauri_plugin_single_instance::init/s,
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

test("mac WDIO automation targets the app executable and performs a real session readiness probe", () => {
  const wdio = fs.readFileSync(WDIO_CONF, "utf8");
  assert.match(wdio, /const resolveWdioApplicationPath = \(appPath\) => \{/);
  assert.match(
    wdio,
    /"tauri:options":\s*\{[\s\S]*application:\s*resolveWdioApplicationPath\(APP_PATH\)/,
  );
  assert.match(
    wdio,
    /before:\s*async\s*\(_capabilities,\s*_specs,\s*browser\)\s*=>\s*\{[\s\S]*await browser\.getWindowHandle\(\);/s,
  );
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
  assert.match(wrapper, /\[desktop-smoke\] CrabNebula backend log: \$\{CTX_AUTOMATION_CN_BACKEND_LOG\}/);
  assert.match(wrapper, /\[desktop-smoke\] CrabNebula driver log: \$\{CTX_AUTOMATION_CN_DRIVER_LOG\}/);
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
