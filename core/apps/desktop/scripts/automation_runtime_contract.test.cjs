const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.resolve(__dirname, "..");
const REPO_ROOT = path.resolve(ROOT, "..", "..", "..");
const TAURI_CARGO = path.join(ROOT, "src-tauri", "Cargo.toml");
const TAURI_MAIN = path.join(ROOT, "src-tauri", "src", "main.rs");
const PACKAGE_JSON = path.join(ROOT, "package.json");
const WDIO_CONF = path.join(ROOT, "automation", "wdio.conf.cjs");
const DESKTOP_SMOKE_WRAPPER = path.join(REPO_ROOT, "scripts", "desktop_smoke_with_infisical.sh");

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

test("first-run local sandbox script defaults to isolated macOS CN backend", () => {
  const pkg = JSON.parse(fs.readFileSync(PACKAGE_JSON, "utf8"));
  assert.match(
    String(pkg.scripts["test:automation:first-run-local-sandbox"] || ""),
    /CTX_AUTOMATION_CN_SHARED_BACKEND=\$\{CTX_AUTOMATION_CN_SHARED_BACKEND:-0\}/,
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
