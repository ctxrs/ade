const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const repoRoot = path.resolve(__dirname, "..", "..");
const scriptPath = path.join(repoRoot, "scripts", "tests", "updater_remote_daemon_e2e.sh");
const scriptText = fs.readFileSync(scriptPath, "utf8");

test("remote updater proof uses isolated WebDriver ports", () => {
  assert.match(scriptText, /PORT_BASE="\$\{CTX_UPDATER_REMOTE_E2E_PORT_BASE:-\$\(\(34000 \+ RANDOM % 20000\)\)\}"/);
  assert.match(scriptText, /TAURI_DRIVER_PORT_VALUE="\$\{CTX_UPDATER_REMOTE_E2E_DRIVER_PORT:-\$\{TAURI_DRIVER_PORT:-\$\{PORT_BASE\}\}\}"/);
  assert.match(scriptText, /TAURI_TEST_BACKEND_PORT_VALUE="\$\{CTX_UPDATER_REMOTE_E2E_BACKEND_PORT:-\$\{TAURI_TEST_BACKEND_PORT:-\$\(\(PORT_BASE \+ 1\)\)\}\}"/);
  assert.match(scriptText, /export TAURI_DRIVER_PORT="\$\{TAURI_DRIVER_PORT_VALUE\}"/);
  assert.match(scriptText, /export TAURI_TEST_BACKEND_PORT="\$\{TAURI_TEST_BACKEND_PORT_VALUE\}"/);
});

test("remote updater proof gives WebDriver enough time to launch published AppImages", () => {
  assert.match(scriptText, /WDIO_CONNECTION_RETRY_TIMEOUT_MS="\$\{CTX_UPDATER_REMOTE_E2E_WDIO_CONNECTION_RETRY_TIMEOUT_MS:-300000\}"/);
  assert.match(
    scriptText,
    /export CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS="\$\{CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS:-\$\{WDIO_CONNECTION_RETRY_TIMEOUT_MS\}\}"/,
  );
});

test("remote updater proof extracts Linux AppImages before handing them to tauri-driver", () => {
  assert.match(scriptText, /resolve_controller_app_for_automation\(\) \{/);
  assert.match(scriptText, /"\$\{app_path\}" --appimage-extract >\/dev\/null/);
  assert.match(scriptText, /local app_run="\$\{app_dir\}\/AppRun"/);
  assert.match(scriptText, /if \[\[ ! -x "\$\{app_run\}" \]\]; then/);
  assert.match(scriptText, /bundle_manifest="\$\(find "\$\{app_dir\}" -type f -path '\*\/bundles\/manifest\.json' -print -quit\)"/);
  assert.match(scriptText, /export APPIMAGE="\$\{app_path\}"/);
  assert.match(scriptText, /export APPDIR="\$\{app_dir\}"/);
  assert.match(scriptText, /export ARGV0="\$\{app_path\}"/);
  assert.match(scriptText, /export CTX_APPIMAGE_PATH="\$\{app_path\}"/);
  assert.match(scriptText, /export CTX_AUTOMATION_SHIPPED_APP_BUNDLES_DIR="\$\(dirname "\$\{bundle_manifest\}"\)"/);
  assert.match(scriptText, /resolved_automation_app_path="\$\(resolve_controller_app_for_automation "\$\{CTX_DESKTOP_APP_PATH\}"\)"/);
  assert.match(scriptText, /export CTX_DESKTOP_APP_PATH="\$\{resolved_automation_app_path\}"/);
});
