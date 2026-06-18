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

test("release proof wrapper allows explicit remote updater proof arch scope", () => {
  assert.match(
    releaseWrapperText,
    /CTX_UPDATER_E2E_ARCHES="\$\{CTX_UPDATER_E2E_ARCHES:-\$\{CTX_RELEASE_REMOTE_UPDATER_PROOF_ARCHES:-linux-x64,linux-arm64\}\}"/,
  );
});

test("remote updater proof uses isolated WebDriver ports", () => {
  assert.match(scriptText, /PORT_BASE="\$\{CTX_UPDATER_REMOTE_E2E_PORT_BASE:-\$\(\(34000 \+ RANDOM % 20000\)\)\}"/);
  assert.match(scriptText, /TAURI_DRIVER_PORT_VALUE="\$\{CTX_UPDATER_REMOTE_E2E_DRIVER_PORT:-\$\{TAURI_DRIVER_PORT:-\$\{PORT_BASE\}\}\}"/);
  assert.match(scriptText, /TAURI_TEST_BACKEND_PORT_VALUE="\$\{CTX_UPDATER_REMOTE_E2E_BACKEND_PORT:-\$\{TAURI_TEST_BACKEND_PORT:-\$\(\(PORT_BASE \+ 1\)\)\}\}"/);
  assert.match(scriptText, /TAURI_DRIVER_NATIVE_PORT_VALUE="\$\{CTX_UPDATER_REMOTE_E2E_NATIVE_DRIVER_PORT:-\$\{TAURI_DRIVER_NATIVE_PORT:-\$\(\(PORT_BASE \+ 2\)\)\}\}"/);
  assert.match(scriptText, /local attempt_driver_port=\$\(\(TAURI_DRIVER_PORT_VALUE \+ \(attempt - 1\) \* 3\)\)/);
  assert.match(scriptText, /local attempt_backend_port=\$\(\(TAURI_TEST_BACKEND_PORT_VALUE \+ \(attempt - 1\) \* 3\)\)/);
  assert.match(scriptText, /local attempt_native_driver_port=\$\(\(TAURI_DRIVER_NATIVE_PORT_VALUE \+ \(attempt - 1\) \* 3\)\)/);
  assert.match(scriptText, /export TAURI_DRIVER_PORT="\$\{attempt_driver_port\}"/);
  assert.match(scriptText, /export TAURI_TEST_BACKEND_PORT="\$\{attempt_backend_port\}"/);
  assert.match(scriptText, /export TAURI_DRIVER_NATIVE_PORT="\$\{attempt_native_driver_port\}"/);
});

test("remote updater proof gives WebDriver enough time to launch published AppImages", () => {
  assert.match(scriptText, /WDIO_CONNECTION_RETRY_TIMEOUT_MS="\$\{CTX_UPDATER_REMOTE_E2E_WDIO_CONNECTION_RETRY_TIMEOUT_MS:-300000\}"/);
  assert.match(
    scriptText,
    /export CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS="\$\{CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS:-\$\{WDIO_CONNECTION_RETRY_TIMEOUT_MS\}\}"/,
  );
});

test("remote updater proof can fail closed instead of falling back to published artifacts", () => {
  assert.match(scriptText, /FORBID_PUBLISHED_FALLBACK="\$\{CTX_UPDATER_REMOTE_E2E_FORBID_PUBLISHED_FALLBACK:-0\}"/);
  assert.match(scriptText, /CTX_UPDATER_REMOTE_E2E_FORBID_PUBLISHED_FALLBACK must be 0 or 1/);
  assert.match(
    scriptText,
    /elif ! resolved_app_path="\$\(resolve_local_smoke_app_path "\$\{ROOT\}"\)"; then\s+if \[\[ "\$FORBID_PUBLISHED_FALLBACK" == "1" \]\]; then\s+echo "error: source\/staged remote proof requires CTX_DESKTOP_APP_PATH or a local AppDir; published artifact fallback is disabled"/,
  );
  assert.match(scriptText, /local AppDir unavailable; downloading published controller AppImage/);
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
  assert.match(scriptText, /export CTX_AUTOMATION_XDG_DIR="\$\{attempt_xdg_dir\}"/);
  assert.match(scriptText, /export CTX_AUTOMATION_CN_BACKEND_LOG="\$\{attempt_dir\}\/crabnebula-backend\.log"/);
  assert.match(scriptText, /export CTX_AUTOMATION_CN_DRIVER_LOG="\$\{attempt_dir\}\/tauri-driver\.log"/);
  assert.match(scriptText, /export CTX_AUTOMATION_APP_LAUNCH_LOG="\$\{attempt_dir\}\/app-launch\.log"/);
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
  assert.match(scriptText, /write_host_resource_snapshot\(\) \{/);
  assert.match(scriptText, /kill_pids_best_effort\(\) \{/);
  assert.match(scriptText, /sudo --non-interactive kill -9/);
  assert.match(scriptText, /sweep_controller_app_processes\(\) \{/);
  assert.match(scriptText, /local app_path="\$\{RESOLVED_CONTROLLER_AUTOMATION_APP_PATH:-\}"/);
  assert.match(scriptText, /app_dir="\$\(cd "\$\(dirname "\$\{app_path\}"\)" 2>\/dev\/null && pwd -P \|\| dirname "\$\{app_path\}"\)"/);
  assert.match(scriptText, /ps -Ao pid=,command=/);
  assert.match(scriptText, /case "\$\{cmd\}" in\s+\*"\$\{app_dir\}"\*\) pids\+=\("\$\{pid\}"\) ;;\s+esac/);
  assert.match(scriptText, /kill_pids_best_effort "\$\{pids\[@\]\}"/);
  assert.match(scriptText, /write_host_resource_snapshot "\$\{attempt_dir\}" "before-sweep"/);
  assert.match(scriptText, /write_host_resource_snapshot "\$\{attempt_dir\}" "after-preflight-sweep"/);
  assert.match(scriptText, /write_host_resource_snapshot "\$\{attempt_dir\}" "after-automation"/);
  assert.match(scriptText, /write_host_resource_snapshot "\$\{attempt_dir\}" "after-automation-sweep"/);
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
  assert.match(scriptText, /write_host_resource_snapshot "\$\{attempt_dir\}" "before-sweep"\s+sweep_stale_xvfb_processes\s+sweep_local_automation_processes/s);
});

test("remote updater proof sweeps exact current attempt Xvfb after automation", () => {
  assert.match(scriptText, /sweep_xvfb_processes_for_tmp_dir\(\) \{/);
  assert.match(scriptText, /resolved_tmp_dir="\$\(cd "\$tmp_dir" 2>\/dev\/null && pwd -P \|\| printf '%s' "\$tmp_dir"\)"/);
  assert.match(scriptText, /ps -Ao pid=,command=/);
  assert.match(scriptText, /\*Xvfb\*"\$\{tmp_dir\}"\* \| \*Xvfb\*"\$\{resolved_tmp_dir\}"\*/);
  assert.match(scriptText, /write_host_resource_snapshot "\$\{attempt_dir\}" "after-automation"\s+sweep_xvfb_processes_for_tmp_dir "\$\{attempt_tmp_dir\}"\s+sweep_stale_xvfb_processes\s+sweep_local_automation_processes/s);
});

test("remote updater proof sweeps local WebKit helpers and e2e daemons between attempts", () => {
  assert.match(scriptText, /sweep_webkit_automation_helpers\(\) \{/);
  assert.match(scriptText, /command_matches_current_native_webkit_port\(\) \{/);
  assert.match(scriptText, /command_is_scoped_webkit_automation_helper\(\) \{/);
  assert.match(scriptText, /command_matches_current_automation_scope\(\) \{/);
  assert.match(scriptText, /command_contains_nonempty_path\(\) \{/);
  assert.match(scriptText, /local native_port="\$\{TAURI_DRIVER_NATIVE_PORT:-\}"/);
  assert.match(scriptText, /\*WebKitWebDriver\*"--port"\*"\$\{native_port\}"\*/);
  assert.match(scriptText, /command_matches_current_native_webkit_port "\$cmd" && return 0/);
  assert.match(scriptText, /\*WebKitWebDriver\*\|\*wkwebdriver\*\|\*WebKitWebProcess\*/);
  assert.match(scriptText, /if ! command_is_scoped_webkit_automation_helper "\$cmd"; then\s+continue\s+fi\s+pids\+=\("\$\{pid\}"\)/);
  assert.match(scriptText, /command_contains_nonempty_path "\$cmd" "\$\{artifact_dir\}"/);
  assert.match(scriptText, /command_contains_nonempty_path "\$cmd" "\$\{CTX_AUTOMATION_TMPDIR:-\}"/);
  assert.match(scriptText, /command_contains_nonempty_path "\$cmd" "\$\{CTX_AUTOMATION_XDG_DIR:-\}"/);
  assert.match(scriptText, /command_contains_nonempty_path "\$cmd" "\$\{app_path\}"/);
  assert.match(scriptText, /sweep_local_automation_daemons\(\) \{/);
  assert.match(scriptText, /sweep_local_automation_daemons\(\) \{[\s\S]*local scoped_daemon_data_dir="\$\{CTX_DESKTOP_DAEMON_DATA_DIR:-\$\{CTX_AUTOMATION_SHIPPED_APP_DAEMON_DATA_DIR:-\}\}"/);
  assert.doesNotMatch(scriptText, /sweep_webkit_automation_helpers\(\) \{[\s\S]*local scoped_daemon_data_dir[\s\S]*sweep_local_automation_daemons\(\) \{/);
  assert.match(scriptText, /\*ctx-daemon\*" serve "\*\) ;;/);
  assert.match(scriptText, /\*"--data-dir "\*"ctx-desktop-e2e-app-daemon-"\*/);
  assert.match(scriptText, /\*"--data-dir "\*"\$\{artifact_dir\}\/automation-attempt-"\*"\/controller-daemon-data"\*/);
  assert.match(scriptText, /\*\"--data-dir \"\*\"\$\{scoped_daemon_data_dir\}\"\* \| \*\"--data-dir=\"\*\"\$\{scoped_daemon_data_dir\}\"\*/);
  assert.match(scriptText, /sweep_local_automation_processes\(\) \{/);
  assert.match(scriptText, /sweep_controller_launch_smoke_processes\(\) \{/);
  assert.match(scriptText, /CTX_AUTOMATION_XDG_DIR="\$\{smoke_xdg_dir\}"/);
  assert.match(scriptText, /CTX_DESKTOP_DAEMON_DATA_DIR="\$\{smoke_dir\}\/controller-daemon-data"/);
  assert.match(scriptText, /sweep_controller_app_processes\s+sweep_webkit_automation_helpers\s+sweep_local_automation_daemons/);
  assert.match(scriptText, /write_host_resource_snapshot "\$\{attempt_dir\}" "before-sweep"[\s\S]*sweep_stale_xvfb_processes[\s\S]*sweep_local_automation_processes[\s\S]*write_host_resource_snapshot "\$\{attempt_dir\}" "after-preflight-sweep"/);
  assert.match(scriptText, /write_host_resource_snapshot "\$\{attempt_dir\}" "after-automation"[\s\S]*sweep_xvfb_processes_for_tmp_dir "\$\{attempt_tmp_dir\}"[\s\S]*sweep_stale_xvfb_processes[\s\S]*sweep_local_automation_processes[\s\S]*write_host_resource_snapshot "\$\{attempt_dir\}" "after-automation-sweep"/);
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

test("remote updater proof writes Linux AppRun launch diagnostics before updater automation", () => {
  assert.match(scriptText, /run_controller_launch_smoke_if_enabled\(\) \{/);
  assert.match(scriptText, /CTX_UPDATER_REMOTE_E2E_CONTROLLER_LAUNCH_SMOKE:-1/);
  assert.match(scriptText, /linux_bundled_launch_smoke\.mjs/);
  assert.match(scriptText, /controller-launch-smoke/);
  assert.match(scriptText, /write_attempt_manifest\(\) \{/);
  assert.match(scriptText, /attempt-manifest\.json/);
  assert.match(scriptText, /native_driver_port: Number\(nativeDriverPort\)/);
  assert.match(scriptText, /app_launch_log: appLaunchLog \|\| null/);
  assert.match(scriptText, /backend_log_expected: platform === "darwin"/);
  assert.match(scriptText, /write_linux_backend_sentinel_if_needed\(\) \{/);
  assert.match(scriptText, /Linux updater proof does not start the CrabNebula test-runner-backend/);
  assert.match(scriptText, /run_controller_launch_smoke_if_enabled/);
  assert.match(scriptText, /write_host_resource_snapshot "\$\{smoke_dir\}" "after"\s+sweep_controller_launch_smoke_processes "\$\{smoke_dir\}" "\$\{smoke_tmp_dir\}" "\$\{smoke_xdg_dir\}"/s);
});

test("remote updater proof retries startup-only WebDriver session failures with preserved logs", () => {
  assert.match(scriptText, /CTX_UPDATER_REMOTE_E2E_AUTOMATION_ATTEMPTS:-2/);
  assert.match(scriptText, /is_retryable_wdio_session_start_failure\(\) \{/);
  assert.match(scriptText, /UND_ERR_HEADERS_TIMEOUT/);
  assert.match(scriptText, /Failed to create a session/);
  assert.match(scriptText, /automation-attempt-\$\{attempt\}/);
  assert.match(scriptText, /attempt_driver_port=\$\(\(TAURI_DRIVER_PORT_VALUE \+ \(attempt - 1\) \* 3\)\)/);
  assert.match(scriptText, /attempt_backend_port=\$\(\(TAURI_TEST_BACKEND_PORT_VALUE \+ \(attempt - 1\) \* 3\)\)/);
  assert.match(scriptText, /attempt_native_driver_port=\$\(\(TAURI_DRIVER_NATIVE_PORT_VALUE \+ \(attempt - 1\) \* 3\)\)/);
  assert.match(scriptText, /export CTX_AUTOMATION_TMPDIR="\$\{attempt_tmp_dir\}"/);
  assert.match(scriptText, /export CTX_AUTOMATION_CN_DRIVER_LOG="\$\{attempt_dir\}\/tauri-driver\.log"/);
  assert.match(scriptText, /export CTX_AUTOMATION_APP_LAUNCH_LOG="\$\{attempt_dir\}\/app-launch\.log"/);
  assert.match(scriptText, /export TAURI_DRIVER_NATIVE_PORT="\$\{attempt_native_driver_port\}"/);
  assert.match(scriptText, /native driver port \$\{TAURI_DRIVER_NATIVE_PORT\}/);
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

test("remote workspace published mode enforces strict published updater proof artifacts", () => {
  const remoteWorkspaceText = fs.readFileSync(
    path.join(repoRoot, "scripts", "buildkite", "run_remote_workspace_e2e.sh"),
    "utf8",
  );

  assert.match(remoteWorkspaceText, /CTX_UPDATER_REMOTE_E2E_STRICT_PUBLISHED_ARTIFACTS="\$\{CTX_UPDATER_REMOTE_E2E_STRICT_PUBLISHED_ARTIFACTS:-1\}"/);
  assert.match(remoteWorkspaceText, /CTX_UPDATER_E2E_STRICT_PUBLISHED_ARTIFACTS="\$\{CTX_UPDATER_E2E_STRICT_PUBLISHED_ARTIFACTS:-1\}"/);
});
