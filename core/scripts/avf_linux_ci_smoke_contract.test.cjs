const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const scriptPath = path.resolve(__dirname, "avf_linux_ci_smoke.sh");
const text = fs.readFileSync(scriptPath, "utf8");

test("avf smoke resolves Cargo target roots from cargo metadata", () => {
  assert.match(text, /cargo_target_dir\(\)/);
  assert.match(text, /cargo metadata --manifest-path "\$manifest" --format-version 1 --no-deps/);
  assert.match(text, /helper_target_root="\$\{CARGO_TARGET_DIR:-\$\(cargo_target_dir "\$helper_manifest"\)\}"/);
  assert.match(text, /guest_target_root="\$\{CARGO_TARGET_DIR:-\$\(cargo_target_dir "\$\{repo_root\}\/Cargo\.toml"\)\}"/);
  assert.doesNotMatch(text, /helper_target_root="\$\{CARGO_TARGET_DIR:-\$\{repo_root\}\/apps\/desktop\/src-tauri\/target\}"/);
});

test("avf smoke supports explicitly skipping restore coverage when the caller only wants guest-exec smoke", () => {
  assert.match(text, /--restore-smoke MODE/);
  assert.match(text, /restore_smoke_mode="required"/);
  assert.match(text, /required\|skip/);
  assert.match(text, /workspace VM save\/restore smoke disabled by --restore-smoke skip/);
});

test("avf smoke drives daemon harness_container ensure with the same shared data_root and bundle dir as the AVF helper", () => {
  assert.match(text, /wait_for_daemon_health\(\)/);
  assert.match(text, /wait_for_daemon_auth_token\(\)/);
  assert.match(text, /tcp_bind_available\(\)/);
  assert.match(text, /ctx_daemon_gateway_required=0/);
  assert.match(text, /ctx_daemon_gateway_required=1/);
  assert.match(text, /run_daemon_harness_container_smoke\(\)/);
  assert.match(text, /node --experimental-websocket - \\/);
  assert.match(text, /Copying canonical runtime lock into staged bundle/);
  assert.match(text, /runtime_lock\.v2\.json/);
  assert.match(text, /CTX_AVF_LINUX_HELPER_PATH="\$helper_bin" \\/);
  assert.match(text, /CTX_BUNDLE_DIR="\$bundle_dir" \\/);
  assert.match(text, /CTX_BUNDLE_MANIFEST="\$bundle_dir\/manifest\.json" \\/);
  assert.match(text, /CTX_RUNTIME_PROFILE=parity \\/);
  assert.match(text, /"\$ctx_bin" serve \\/);
  assert.match(text, /daemon_bind_args=\(--bind "\$ctx_daemon_loopback_bind"\)/);
  assert.match(text, /--data-dir "\$data_root"/);
  assert.match(text, /daemon_auth_file="\$\{data_root\}\/daemon_auth\.json"/);
  assert.match(text, /daemon_auth_token="\$\(wait_for_daemon_auth_token "\$daemon_auth_file"\)"/);
  assert.match(text, /\/api\/workspaces\/\$\{report\.workspace\.id\}\/harness_container\/ensure/);
  assert.match(text, /\/api\/workspaces\/\$\{report\.workspace\.id\}\/tasks/);
  assert.match(text, /\/api\/worktrees\/\$\{taskResp\.json\.primary_worktree_id\}/);
  assert.match(text, /\/api\/workspaces\/\$\{report\.workspace\.id\}\/terminals/);
  assert.match(text, /authorization: `Bearer \$\{authToken\}`/);
  assert.match(text, /wsPath\.searchParams\.set\("token", authToken\)/);
  assert.match(text, /workspaceResp\.json\.id !== requestedWorkspaceId\.trim\(\)/);
  assert.match(text, /task_id: taskResp\.json\.id/);
  assert.match(text, /worktree_id: taskResp\.json\.primary_worktree_id/);
  assert.match(text, /terminalResp\.json\.cwd !== worktreeResp\.json\.root_path/);
  assert.match(text, /terminalResp\.json\.task_id !== taskResp\.json\.id/);
  assert.match(text, /terminalResp\.json\.worktree_id !== taskResp\.json\.primary_worktree_id/);
  assert.match(text, /runTerminalSmoke\(terminalId, worktreeResp\.json\.root_path\)/);
  assert.match(text, /text\.includes\(expectedWorktreeRoot\)/);
  assert.match(text, /startsWith\("\/ctx\/ws\/worktrees\/"\)/);
  assert.match(text, /harness_container_name="\$\(json_get "\$daemon_smoke_json" 'input\.harness_container\.name'\)"/);
  assert.match(text, /restore_harness_container_name="\$\(json_get "\$daemon_smoke_restore_json" 'input\.harness_container\.name'\)"/);
  assert.match(text, /recreated the AVF harness container across restore/);
  assert.match(text, /AVF guest gateway bind unavailable on host; skipping direct guest daemon reachability smoke/);
  assert.match(text, /AVF guest gateway bind unavailable on host; skipping restricted-network guest daemon reachability smoke/);
  assert.match(text, /harness_container_status="skipped"/);
  assert.match(text, /avf_guest_gateway_required: guestGatewayRequired === "1"/);
  assert.match(text, /daemon_harness_container: maybeJson\(daemonSmokePath\)/);
  assert.match(text, /harness_container: \{\s*status: harnessContainerStatus,\s*reason: harnessContainerReason,/s);
});

test("avf smoke clears stale artifacts and uses structured restore outcomes", () => {
  assert.match(text, /cleanup_artifact_dir\(\)/);
  assert.match(text, /cleanup_artifact_dir "\$artifact_dir"/);
  assert.match(text, /restore_start_outcome="\$\(json_get "\$start2_json" 'input\.last_start_outcome'/);
  assert.doesNotMatch(text, /grep -q "restored workspace VM state"/);
});
