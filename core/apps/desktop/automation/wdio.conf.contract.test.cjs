#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const configPath = path.join(__dirname, "wdio.conf.cjs");
const escapeRegExp = (value) => String(value).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

test("wdio automation defaults route runtime and daemon scratch through the volatile root", () => {
  const script = fs.readFileSync(configPath, "utf8");

  assert.match(script, /resolveVolatileRoot = \(\) =>/);
  assert.match(script, /path\.join\(os\.homedir\(\), "\.ctx", "volatile"\)/);
  assert.match(script, /AUTOMATION_ARTIFACTS_ROOT = resolveVolatileSubdir\("CTX_VOLATILE_ARTIFACTS_DIR", \["artifacts"\]\)/);
  assert.match(script, /resolveAutomationTmpRoot = \(\) =>/);
  assert.match(script, /process\.env\.TMPDIR = automationTmpDir/);
  assert.match(script, /process\.env\.TAURI_WEBVIEW_AUTOMATION = "true"/);
  assert.match(script, /return path\.join\(AUTOMATION_ARTIFACTS_ROOT, "ctx-desktop-e2e", "avf-linux-guest-runtime"\)/);
  assert.match(script, /fs\.mkdtempSync\(path\.join\(automationTmpDir, `ctx-desktop-e2e-\$\{daemonPort\}-`\)\)/);
  assert.match(script, /fs\.mkdtempSync\(path\.join\(automationTmpDir, "ctx-desktop-e2e-app-daemon-"\)\)/);
});

test("wdio shipped-app mode is cross-platform and supports an explicit bundle dir override", () => {
  const script = fs.readFileSync(configPath, "utf8");

  assert.match(script, /const SHIPPED_APP_BUNDLES_DIR_OVERRIDE = resolveConfiguredPath\(\s*process\.env\.CTX_AUTOMATION_SHIPPED_APP_BUNDLES_DIR,\s*\)/);
  assert.match(script, /const SHIPPED_APP_DAEMON_DATA_DIR_OVERRIDE = resolveConfiguredPath\(\s*process\.env\.CTX_AUTOMATION_SHIPPED_APP_DAEMON_DATA_DIR,\s*\)/);
  assert.match(script, /const USING_SHIPPED_APP_MODE = SHIPPED_APP_MODE;/);
  assert.match(script, /if \(USING_SHIPPED_APP_MODE\) \{\s*if \(SHIPPED_APP_BUNDLES_DIR_OVERRIDE\) \{\s*return SHIPPED_APP_BUNDLES_DIR_OVERRIDE;/s);
  assert.match(
    script,
    /if \(USING_SHIPPED_APP_MODE\) \{\s*if \(SHIPPED_APP_DAEMON_DATA_DIR_OVERRIDE\) \{\s*fs\.mkdirSync\(SHIPPED_APP_DAEMON_DATA_DIR_OVERRIDE, \{ recursive: true \}\);\s*internalDaemonDataDir = canonicalPath\(SHIPPED_APP_DAEMON_DATA_DIR_OVERRIDE\);/s,
  );
  assert.match(
    script,
    /CTX_AUTOMATION_SHIPPED_APP=1 requires CTX_AUTOMATION_SHIPPED_APP_BUNDLES_DIR for Linux\/Windows container scenarios/,
  );
  assert.match(
    script,
    /delete process\.env\.CTX_DESKTOP_DEV_BIN_DIR;[\s\S]*delete process\.env\.CTX_DESKTOP_START_PATH;/,
  );
});

test("wdio macOS shipped-app launches through a wrapper with exact app env", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-launch-env-"));
  try {
    const appPath = path.join(tmp, "ctx-real");
    const bundlesDir = path.join(tmp, "bundles");
    const daemonDataDir = path.join(tmp, "workspace-home", ".ctx");
    const appDir = path.join(tmp, "squashfs-root");
    const appImage = path.join(tmp, "ctx.AppImage");
    const runtimeDir = path.join(tmp, "xdg-runtime");
    fs.writeFileSync(appPath, "#!/bin/sh\nexit 0\n", { mode: 0o700 });
    fs.mkdirSync(bundlesDir, { recursive: true });
    fs.mkdirSync(appDir, { recursive: true });
    fs.mkdirSync(runtimeDir, { recursive: true });

    const nodeScript = [
      `Object.defineProperty(process, "platform", { value: "darwin" });`,
      `const cfg = require(${JSON.stringify(configPath)});`,
      `process.stdout.write(cfg.config.capabilities[0]["tauri:options"].application);`,
    ].join("\n");
    const wrapperPath = execFileSync(process.execPath, ["-e", nodeScript], {
      cwd: path.resolve(__dirname, "..", "..", ".."),
      encoding: "utf8",
      env: {
        ...process.env,
        CTX_AUTOMATION_TMPDIR: tmp,
        CTX_DESKTOP_APP_PATH: appPath,
        CTX_AUTOMATION_SHIPPED_APP: "1",
        CTX_AUTOMATION_SHIPPED_APP_BUNDLES_DIR: bundlesDir,
        CTX_AUTOMATION_SHIPPED_APP_DAEMON_DATA_DIR: daemonDataDir,
        CTX_AUTOMATION_SSH_NO_START_REMOTE: "0",
        APPDIR: appDir,
        APPIMAGE: appImage,
        APPIMAGE_EXTRACT_AND_RUN: "1",
        ARGV0: appImage,
        CTX_APPIMAGE_PATH: appImage,
        DISPLAY: ":99",
        XDG_RUNTIME_DIR: runtimeDir,
      },
    });
    assert.notEqual(wrapperPath, appPath);
    assert.equal(path.dirname(wrapperPath), path.join(tmp, "desktop-app-launchers"));
    const wrapper = fs.readFileSync(wrapperPath, "utf8");
    assert.match(wrapper, new RegExp(`export CTX_BUNDLE_DIR='${escapeRegExp(bundlesDir)}'`));
    assert.match(
      wrapper,
      new RegExp(`export CTX_DESKTOP_DAEMON_DATA_DIR='${escapeRegExp(daemonDataDir)}'`),
    );
    assert.match(wrapper, /export CTX_DESKTOP_SSH_NO_START_REMOTE='0'/);
    assert.match(wrapper, /export CTX_DESKTOP_SSH_START_REMOTE='1'/);
    assert.match(wrapper, /export TAURI_WEBVIEW_AUTOMATION='true'/);
    assert.match(wrapper, new RegExp(`export APPDIR='${escapeRegExp(appDir)}'`));
    assert.match(wrapper, new RegExp(`export APPIMAGE='${escapeRegExp(appImage)}'`));
    assert.match(wrapper, /export APPIMAGE_EXTRACT_AND_RUN='1'/);
    assert.match(wrapper, new RegExp(`export ARGV0='${escapeRegExp(appImage)}'`));
    assert.match(wrapper, new RegExp(`export CTX_APPIMAGE_PATH='${escapeRegExp(appImage)}'`));
    assert.match(wrapper, /export DISPLAY=':99'/);
    assert.match(wrapper, new RegExp(`export XDG_RUNTIME_DIR='${escapeRegExp(runtimeDir)}'`));
    assert.match(wrapper, /launch env TAURI_WEBVIEW_AUTOMATION=/);
    assert.match(wrapper, new RegExp(`exec '${escapeRegExp(appPath)}' "\\$@"`));
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test("wdio Linux shipped-app passes raw AppRun while tauri-driver carries AppDir env", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-linux-launch-env-"));
  try {
    const bundlesDir = path.join(tmp, "bundles");
    const daemonDataDir = path.join(tmp, "workspace-home", ".ctx");
    const appDir = path.join(tmp, "squashfs-root");
    const appPath = path.join(appDir, "AppRun");
    const appImage = path.join(tmp, "ctx.AppImage");
    const runtimeDir = path.join(tmp, "xdg-runtime");
    const innerBinary = path.join(appDir, "usr", "bin", "ctx");
    fs.mkdirSync(path.dirname(innerBinary), { recursive: true });
    fs.writeFileSync(appPath, "#!/bin/sh\nexit 0\n", { mode: 0o700 });
    fs.writeFileSync(innerBinary, "#!/bin/sh\nexit 0\n", { mode: 0o700 });
    fs.mkdirSync(bundlesDir, { recursive: true });
    fs.mkdirSync(runtimeDir, { recursive: true });

    const nodeScript = [
      `Object.defineProperty(process, "platform", { value: "linux" });`,
      `const cfg = require(${JSON.stringify(configPath)});`,
      `process.stdout.write(cfg.config.capabilities[0]["tauri:options"].application);`,
    ].join("\n");
    const resolvedPath = execFileSync(process.execPath, ["-e", nodeScript], {
      cwd: path.resolve(__dirname, "..", "..", ".."),
      encoding: "utf8",
      env: {
        ...process.env,
        CTX_AUTOMATION_TMPDIR: tmp,
        CTX_DESKTOP_APP_PATH: appPath,
        CTX_AUTOMATION_SHIPPED_APP: "1",
        CTX_AUTOMATION_SHIPPED_APP_BUNDLES_DIR: bundlesDir,
        CTX_AUTOMATION_SHIPPED_APP_DAEMON_DATA_DIR: daemonDataDir,
        CTX_AUTOMATION_SSH_NO_START_REMOTE: "0",
        APPDIR: appDir,
        APPIMAGE: appImage,
        APPIMAGE_EXTRACT_AND_RUN: "1",
        ARGV0: appImage,
        CTX_APPIMAGE_PATH: appImage,
        DISPLAY: ":99",
        XDG_RUNTIME_DIR: runtimeDir,
      },
    });
    assert.equal(resolvedPath, appPath);
    assert.ok(!fs.existsSync(path.join(tmp, "desktop-app-launchers")));

    const script = fs.readFileSync(configPath, "utf8");
    assert.match(script, /buildLinuxAppDirLaunchEnv/);
    assert.match(script, /createLinuxAppDirLaunchWrapper/);
    assert.match(script, /if \(process\.platform === "linux"\) \{[\s\S]*return appExecutablePath;[\s\S]*\}/);
    assert.match(script, /const driverEnv = \{\s*\.\.\.process\.env,/);
    assert.match(script, /\.\.\.DESKTOP_APP_LAUNCH_ENV,/);
    assert.match(script, /console\.error\(`\[wdio\] WebDriver application path=\$\{WDIO_APPLICATION_PATH\}`\)/);
    assert.match(script, /TAURI_DRIVER_PORT: String\(activeTauriDriverPort\)/);
    assert.match(script, /TAURI_DRIVER_NATIVE_PORT: String\(activeTauriDriverNativePort\)/);
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test("wdio remote-only prep on mac arm64 skips the managed AVF payload download", () => {
  const script = fs.readFileSync(configPath, "utf8");

  assert.match(script, /const RUNS_REMOTE_ONLY_SCENARIOS = SCENARIO_FILTER\.length > 0/);
  assert.match(script, /SCENARIO_FILTER\.every\(\(token\) => token\.startsWith\("remote"\)\)/);
  assert.match(
    script,
    /process\.platform === "darwin"[\s\S]*process\.arch === "arm64"[\s\S]*RUNS_REMOTE_ONLY_SCENARIOS[\s\S]*CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD = "1"/,
  );
});

test("wdio explicit local-only scenario prep disables bundled remote daemons", () => {
  const script = fs.readFileSync(configPath, "utf8");

  assert.match(script, /const RUNS_REMOTE_SCENARIOS = SCENARIO_FILTER\.length === 0/);
  assert.match(script, /SCENARIO_FILTER\.some\(\(token\) => token\.startsWith\("remote"\)\)/);
  assert.match(
    script,
    /if \(!RUNS_REMOTE_SCENARIOS && !String\(prepEnv\.CTX_BUNDLE_REMOTE_DAEMONS \|\| ""\)\.trim\(\)\) \{\s*prepEnv\.CTX_BUNDLE_REMOTE_DAEMONS = "0";\s*\}/s,
  );
});

test("wdio pure host local scenario prep disables Linux ctx-mcp bundling", () => {
  const script = fs.readFileSync(configPath, "utf8");

  assert.match(script, /const RUNS_CONTAINER_SCENARIOS = SCENARIO_FILTER\.length === 0/);
  assert.match(script, /SCENARIO_FILTER\.some\(\(token\) => CONTAINER_SCENARIO_TOKENS\.has\(token\)\)/);
  assert.match(
    script,
    /if \(\s*!RUNS_CONTAINER_SCENARIOS\s*&& !RUNS_REMOTE_SCENARIOS\s*&& !String\(prepEnv\.CTX_BUNDLE_LINUX_CTX_MCP_RUNTIME \|\| ""\)\.trim\(\)\s*\) \{\s*prepEnv\.CTX_BUNDLE_LINUX_CTX_MCP_RUNTIME = "0";\s*\}/s,
  );
});

test("wdio container automation uses staged AVF runtime instead of copying it into the bundle", () => {
  const script = fs.readFileSync(configPath, "utf8");

  assert.match(
    script,
    /process\.platform === "darwin"[\s\S]*process\.arch === "arm64"[\s\S]*RUNS_CONTAINER_SCENARIOS[\s\S]*CTX_AVF_LINUX_GUEST_RUNTIME_DIR[\s\S]*CTX_DESKTOP_STAGE_AVF_LINUX_GUEST_RUNTIME = "0"/,
  );
});

test("wdio shared CrabNebula backend launch env includes webview automation enablement", () => {
  const script = fs.readFileSync(configPath, "utf8");

  assert.match(script, /const SHARED_CN_BACKEND_ENV_KEYS = new Set\(\[/);
  assert.match(script, /"TAURI_WEBVIEW_AUTOMATION"/);
});

test("wdio connection retries are configurable for packaged mac app readiness", () => {
  const script = fs.readFileSync(configPath, "utf8");

  assert.match(script, /const resolveConnectionRetryCount = \(\) => parsePositiveInt\(/);
  assert.match(script, /CTX_AUTOMATION_CONNECTION_RETRY_COUNT/);
  assert.match(script, /CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS/);
  assert.match(script, /connectionRetryCount: CONNECTION_RETRY_COUNT/);
  assert.match(script, /connectionRetryTimeout: CONNECTION_RETRY_TIMEOUT_MS/);
});

test("wdio chooses an unused tauri-driver port on every platform by default", () => {
  const script = fs.readFileSync(configPath, "utf8");

  assert.match(script, /const DEFAULT_DRIVER_PORT = pickUnusedPortSync\(4444\);/);
  assert.match(script, /const DEFAULT_NATIVE_DRIVER_PORT = pickUnusedPortExcludingSync\(4445, new Set\(\[TAURI_DRIVER_PORT\]\)\);/);
  assert.match(script, /TAURI_DRIVER_NATIVE_PORT/);
  assert.match(script, /native_driver\(requested\)=/);
  assert.match(script, /nativePort: activeTauriDriverNativePort/);
  assert.doesNotMatch(script, /process\.platform === "darwin"\s*\?\s*pickUnusedPortSync\(4444\)\s*:\s*4444/);
});

test("wdio completion reaps current tauri-driver and xvfb infrastructure", () => {
  const script = fs.readFileSync(configPath, "utf8");

  assert.match(script, /const killCurrentAutomationInfrastructure = \(\) => \{/);
  assert.match(script, /commandLooksLikeTauriDriver\(cmd\) && commandHasPortArg\(cmd, activeTauriDriverPort\)/);
  assert.match(script, /process\.platform !== "darwin" && \S+Xvfb\S+\.test\(cmd\) && cmd\.includes\(currentTmpDir\)/);
  assert.match(script, /killCurrentAutomationInfrastructure\(\);[\s\S]*if \(ALLOW_STALE_HELPER_SWEEP && !usesSharedCnBackend\)/);
});
