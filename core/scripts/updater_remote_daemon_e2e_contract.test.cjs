const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const repoRoot = path.resolve(__dirname, "..", "..");
const scriptPath = path.join(repoRoot, "scripts", "tests", "updater_remote_daemon_e2e.sh");
const scriptText = fs.readFileSync(scriptPath, "utf8");
const releaseWrapperText = fs.readFileSync(
  path.join(repoRoot, "scripts", "buildkite", "run_release_proof_remote_updater.sh"),
  "utf8",
);

test("remote updater proof uses isolated WebDriver ports", () => {
  assert.match(scriptText, /PORT_BASE="\$\{CTX_UPDATER_REMOTE_E2E_PORT_BASE:-\$\(\(34000 \+ RANDOM % 20000\)\)\}"/);
  assert.match(scriptText, /TAURI_DRIVER_PORT_VALUE="\$\{CTX_UPDATER_REMOTE_E2E_DRIVER_PORT:-\$\{TAURI_DRIVER_PORT:-\$\{PORT_BASE\}\}\}"/);
  assert.match(scriptText, /TAURI_TEST_BACKEND_PORT_VALUE="\$\{CTX_UPDATER_REMOTE_E2E_BACKEND_PORT:-\$\{TAURI_TEST_BACKEND_PORT:-\$\(\(PORT_BASE \+ 1\)\)\}\}"/);
  assert.match(scriptText, /local attempt_driver_port=\$\(\(TAURI_DRIVER_PORT_VALUE \+ \(attempt - 1\) \* 2\)\)/);
  assert.match(scriptText, /local attempt_backend_port=\$\(\(TAURI_TEST_BACKEND_PORT_VALUE \+ \(attempt - 1\) \* 2\)\)/);
  assert.match(scriptText, /export TAURI_DRIVER_PORT="\$\{attempt_driver_port\}"/);
  assert.match(scriptText, /export TAURI_TEST_BACKEND_PORT="\$\{attempt_backend_port\}"/);
});

test("remote updater proof gives WebDriver enough time to launch published AppImages", () => {
  assert.match(scriptText, /WDIO_CONNECTION_RETRY_TIMEOUT_MS="\$\{CTX_UPDATER_REMOTE_E2E_WDIO_CONNECTION_RETRY_TIMEOUT_MS:-300000\}"/);
  assert.match(
    scriptText,
    /export CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS="\$\{CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS:-\$\{WDIO_CONNECTION_RETRY_TIMEOUT_MS\}\}"/,
  );
});

test("remote updater proof keeps shipped-app automation state inside the artifact directory", () => {
  assert.match(scriptText, /local attempt_dir="\$\{artifact_dir\}\/automation-attempt-\$\{attempt\}"/);
  assert.match(scriptText, /local attempt_tmp_dir="\$\{attempt_dir\}\/tmp"/);
  assert.match(scriptText, /attempt_xdg_token="\$\(printf '%s' "\$\{BUILDKITE_JOB_ID:-local\}-\$\{attempt\}-\$\$" \| tr -c 'A-Za-z0-9\._-' '_'\)"/);
  assert.match(scriptText, /local attempt_xdg_dir="\/tmp\/ctx-updater-remote-xdg-\$\{attempt_xdg_token\}"/);
  assert.match(scriptText, /local attempt_xdg_runtime_dir="\$\{attempt_xdg_dir\}\/runtime"/);
  assert.match(scriptText, /local attempt_home_dir="\$\{attempt_xdg_dir\}\/home"/);
  assert.match(scriptText, /local attempt_corepack_home="\$\{COREPACK_HOME:-\$\{ORIGINAL_HOME\}\/\.cache\/node\/corepack\}"/);
  assert.match(scriptText, /export CTX_AUTOMATION_TMPDIR="\$\{attempt_tmp_dir\}"/);
  assert.match(scriptText, /export CTX_AUTOMATION_CN_BACKEND_LOG="\$\{attempt_dir\}\/crabnebula-backend\.log"/);
  assert.match(scriptText, /export CTX_AUTOMATION_CN_DRIVER_LOG="\$\{attempt_dir\}\/tauri-driver\.log"/);
  assert.match(scriptText, /export CTX_AUTOMATION_SHIPPED_APP_DAEMON_DATA_DIR="\$\{attempt_dir\}\/controller-daemon-data"/);
  assert.match(scriptText, /export HOME="\$\{attempt_home_dir\}"/);
  assert.match(scriptText, /export XDG_RUNTIME_DIR="\$\{attempt_xdg_runtime_dir\}"/);
  assert.match(scriptText, /export XDG_CONFIG_HOME="\$\{attempt_xdg_dir\}\/config"/);
  assert.match(scriptText, /export XDG_CACHE_HOME="\$\{attempt_xdg_dir\}\/cache"/);
  assert.match(scriptText, /export XDG_DATA_HOME="\$\{attempt_xdg_dir\}\/data"/);
  assert.match(scriptText, /export COREPACK_HOME="\$\{attempt_corepack_home\}"/);
  assert.match(scriptText, /rm -rf "\$\{attempt_xdg_dir\}"/);
  assert.match(scriptText, /if \[\[ "\$status" -eq 0 \]\]; then\s+rm -rf "\$\{attempt_xdg_dir\}"/);
  assert.match(scriptText, /is_retryable_wdio_session_start_failure "\$attempt_log"; then\s+rm -rf "\$\{attempt_xdg_dir\}"/);
  assert.match(scriptText, /fi\s+rm -rf "\$\{attempt_xdg_dir\}"\s+return "\$status"/);
  assert.match(scriptText, /"\$\{attempt_tmp_dir\}"/);
  assert.match(scriptText, /"\$\{HOME\}"/);
  assert.match(scriptText, /"\$\{XDG_RUNTIME_DIR\}"/);
  assert.match(scriptText, /chmod 700 "\$\{XDG_RUNTIME_DIR\}"/);
});

test("remote updater proof sweeps scoped AppImage helper processes between attempts", () => {
  assert.match(scriptText, /write_process_snapshot\(\) \{/);
  assert.match(scriptText, /sweep_controller_app_processes\(\) \{/);
  assert.match(scriptText, /local app_path="\$\{RESOLVED_CONTROLLER_AUTOMATION_APP_PATH:-\}"/);
  assert.match(scriptText, /app_dir="\$\(cd "\$\(dirname "\$\{app_path\}"\)" 2>\/dev\/null && pwd -P \|\| dirname "\$\{app_path\}"\)"/);
  assert.match(scriptText, /ps -Ao pid=,command=/);
  assert.match(scriptText, /case "\$\{cmd\}" in\s+\*"\$\{app_dir\}"\*\) pids\+=\("\$\{pid\}"\) ;;\s+esac/);
  assert.match(scriptText, /kill -9 "\$\{pids\[@\]\}"/);
  assert.match(scriptText, /write_process_snapshot "\$\{attempt_dir\}\/processes-before-sweep\.log"/);
  assert.match(scriptText, /write_process_snapshot "\$\{attempt_dir\}\/processes-after-preflight-sweep\.log"/);
  assert.match(scriptText, /write_process_snapshot "\$\{attempt_dir\}\/processes-after-automation\.log"/);
  assert.match(scriptText, /write_process_snapshot "\$\{attempt_dir\}\/processes-after-automation-sweep\.log"/);
});

test("remote updater proof preflight sweeps stale automation Xvfb processes", () => {
  assert.match(scriptText, /process_elapsed_seconds\(\) \{/);
  assert.match(scriptText, /command_is_ctx_automation_xvfb\(\) \{/);
  assert.match(scriptText, /sweep_stale_xvfb_processes\(\) \{/);
  assert.match(scriptText, /CTX_UPDATER_REMOTE_E2E_SWEEP_STALE_XVFB:-1/);
  assert.match(scriptText, /CTX_UPDATER_REMOTE_E2E_STALE_XVFB_MIN_AGE_SECONDS:-900/);
  assert.match(scriptText, /ps -Ao pid=,ppid=,etime=,command=/);
  assert.match(scriptText, /"\$ppid" != "1"/);
  assert.match(scriptText, /command_is_ctx_automation_xvfb "\$cmd"/);
  assert.match(scriptText, /process_elapsed_seconds "\$elapsed"/);
  assert.match(scriptText, /"\$age_seconds" -lt "\$min_age_seconds"/);
  assert.match(scriptText, /\*Xvfb\*\/ctx-nightly\/\.artifacts\/buildkite\/ctx-nightly\/\*\/updater-proof\/automation-attempt-\*\/tmp\/xvfb-run\.\*\/Xauthority\*/);
  assert.match(scriptText, /\*Xvfb\*\/core\/apps\/desktop\/automation\/artifacts\/updater-remote-proof\/automation-attempt-\*\/tmp\/xvfb-run\.\*\/Xauthority\*/);
  assert.match(scriptText, /\*Xvfb\*\/core\/apps\/desktop\/automation\/artifacts\/updater-linux-proof\/\*\/volatile\/artifacts\/ctx-desktop-e2e\/\*\/xvfb-run\.\*\/Xauthority\*/);
  assert.match(scriptText, /\*Xvfb\*\/\.ctx\/volatile\/artifacts\/ctx-desktop-e2e\/\*\/xvfb-run\.\*\/Xauthority\*/);
  assert.match(scriptText, /write_process_snapshot "\$\{attempt_dir\}\/processes-before-sweep\.log"\s+sweep_stale_xvfb_processes\s+sweep_local_automation_processes/s);
});

test("remote updater proof sweeps exact current attempt Xvfb after automation", () => {
  assert.match(scriptText, /sweep_xvfb_processes_for_tmp_dir\(\) \{/);
  assert.match(scriptText, /resolved_tmp_dir="\$\(cd "\$tmp_dir" 2>\/dev\/null && pwd -P \|\| printf '%s' "\$tmp_dir"\)"/);
  assert.match(scriptText, /ps -Ao pid=,command=/);
  assert.match(scriptText, /\*Xvfb\*"\$\{tmp_dir\}"\* \| \*Xvfb\*"\$\{resolved_tmp_dir\}"\*/);
  assert.match(scriptText, /write_process_snapshot "\$\{attempt_dir\}\/processes-after-automation\.log"\s+sweep_xvfb_processes_for_tmp_dir "\$\{attempt_tmp_dir\}"\s+sweep_stale_xvfb_processes\s+sweep_local_automation_processes/s);
});

test("remote updater proof sweeps local WebKit helpers and e2e daemons between attempts", () => {
  assert.match(scriptText, /sweep_webkit_automation_helpers\(\) \{/);
  assert.match(scriptText, /\*WebKitWebDriver\*\|\*wkwebdriver\*\|\*WebKitWebProcess\*/);
  assert.match(scriptText, /sweep_local_automation_daemons\(\) \{/);
  assert.match(scriptText, /\*ctx-daemon\*" serve "\*\) ;;/);
  assert.match(scriptText, /\*"--data-dir "\*"ctx-desktop-e2e-app-daemon-"\*/);
  assert.match(scriptText, /\*"--data-dir "\*"\$\{artifact_dir\}\/automation-attempt-"\*"\/controller-daemon-data"\*/);
  assert.match(scriptText, /sweep_local_automation_processes\(\) \{/);
  assert.match(scriptText, /sweep_controller_app_processes\s+sweep_webkit_automation_helpers\s+sweep_local_automation_daemons/);
  assert.match(scriptText, /write_process_snapshot "\$\{attempt_dir\}\/processes-before-sweep\.log"[\s\S]*sweep_stale_xvfb_processes[\s\S]*sweep_local_automation_processes[\s\S]*write_process_snapshot "\$\{attempt_dir\}\/processes-after-preflight-sweep\.log"/);
  assert.match(scriptText, /write_process_snapshot "\$\{attempt_dir\}\/processes-after-automation\.log"[\s\S]*sweep_xvfb_processes_for_tmp_dir "\$\{attempt_tmp_dir\}"[\s\S]*sweep_stale_xvfb_processes[\s\S]*sweep_local_automation_processes[\s\S]*write_process_snapshot "\$\{attempt_dir\}\/processes-after-automation-sweep\.log"/);
});

test("remote updater proof sweeps stale automation egress proxies", () => {
  assert.match(scriptText, /command_is_ctx_automation_egress_proxy\(\) \{/);
  assert.match(scriptText, /sweep_stale_egress_proxy_processes\(\) \{/);
  assert.match(scriptText, /CTX_UPDATER_REMOTE_E2E_SWEEP_STALE_EGRESS_PROXY:-1/);
  assert.match(scriptText, /CTX_UPDATER_REMOTE_E2E_STALE_EGRESS_PROXY_MIN_AGE_SECONDS:-900/);
  assert.match(scriptText, /ps -Ao pid=,etime=,command=/);
  assert.match(scriptText, /ctx-egress-proxy\*" --config "\*\/core\/apps\/desktop\/automation\/artifacts\/updater-linux-proof\/\*\/workspace-home\/\.ctx\/containers\/workspaces\/\*\/data\/egress-proxy\.json/);
  assert.match(scriptText, /ctx-egress-proxy\*" --config "\*\/ctx-nightly\/\.artifacts\/buildkite\/ctx-nightly\/\*\/updater-proof\/\*\/\.ctx\/containers\/workspaces\/\*\/data\/egress-proxy\.json/);
  assert.match(scriptText, /\*"\$\{artifact_dir\}"\*\) continue ;;/);
  assert.match(scriptText, /process_elapsed_seconds "\$elapsed"/);
  assert.match(scriptText, /sweep_controller_app_processes\s+sweep_webkit_automation_helpers\s+sweep_local_automation_daemons\s+sweep_stale_egress_proxy_processes/);
});

test("remote updater proof retries startup-only WebDriver session failures with preserved logs", () => {
  assert.match(scriptText, /CTX_UPDATER_REMOTE_E2E_AUTOMATION_ATTEMPTS:-2/);
  assert.match(scriptText, /is_retryable_wdio_session_start_failure\(\) \{/);
  assert.match(scriptText, /UND_ERR_HEADERS_TIMEOUT/);
  assert.match(scriptText, /Failed to create a session/);
  assert.match(scriptText, /automation-attempt-\$\{attempt\}/);
  assert.match(scriptText, /attempt_driver_port=\$\(\(TAURI_DRIVER_PORT_VALUE \+ \(attempt - 1\) \* 2\)\)/);
  assert.match(scriptText, /attempt_backend_port=\$\(\(TAURI_TEST_BACKEND_PORT_VALUE \+ \(attempt - 1\) \* 2\)\)/);
  assert.match(scriptText, /export CTX_AUTOMATION_TMPDIR="\$\{attempt_tmp_dir\}"/);
  assert.match(scriptText, /export CTX_AUTOMATION_CN_DRIVER_LOG="\$\{attempt_dir\}\/tauri-driver\.log"/);
  assert.match(scriptText, /tee "\$attempt_log"/);
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
  assert.match(scriptText, /RESOLVED_CONTROLLER_AUTOMATION_APP_PATH=""/);
  assert.match(scriptText, /RESOLVED_CONTROLLER_AUTOMATION_APP_PATH="\$\{app_path\}"/);
  assert.match(scriptText, /RESOLVED_CONTROLLER_AUTOMATION_APP_PATH="\$\{app_run\}"/);
  assert.match(scriptText, /resolve_controller_app_for_automation "\$\{CTX_DESKTOP_APP_PATH\}"/);
  assert.doesNotMatch(scriptText, /resolve_controller_app_for_automation "\$\{CTX_DESKTOP_APP_PATH\}"\)/);
  assert.match(scriptText, /export CTX_DESKTOP_APP_PATH="\$\{RESOLVED_CONTROLLER_AUTOMATION_APP_PATH\}"/);
});

test("release remote updater proof consumes strict published artifacts", () => {
  assert.match(releaseWrapperText, /CTX_UPDATER_E2E_STRICT_PUBLISHED_ARTIFACTS=1/);
  assert.match(releaseWrapperText, /CTX_UPDATER_REMOTE_E2E_STRICT_PUBLISHED_ARTIFACTS=1/);
  assert.match(releaseWrapperText, /CTX_UPDATER_E2E_BOOTSTRAP_REMOTE_MIGRATIONS=0/);
  assert.match(releaseWrapperText, /bash \.\/scripts\/tests\/updater_remote_daemon_e2e\.sh/);
});
