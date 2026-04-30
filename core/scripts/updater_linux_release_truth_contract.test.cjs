const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const repoRoot = path.resolve(__dirname, "..", "..");
const scriptPath = path.join(repoRoot, "scripts", "tests", "updater_linux_release_truth.sh");
const scriptText = fs.readFileSync(scriptPath, "utf8");

function read(relativePath) {
  return fs.readFileSync(path.join(repoRoot, relativePath), "utf8");
}

test("Linux updater proof removes only stale proof daemon locks between automation phases", () => {
  assert.match(scriptText, /remove_stale_proof_lock_if_dead\(\) \{/);
  assert.match(
    scriptText,
    /if \[\[ -n "\$\{lock_pid\}" && "\$\{lock_pid\}" =~ \^\[0-9\]\+\$ \]\]; then\s+if kill -0 "\$\{lock_pid\}" 2>\/dev\/null; then\s+return 0\s+fi\s+fi/,
  );
  assert.match(scriptText, /rm -f "\$\{lock_file\}"/);

  const workspaceProofIndex = scriptText.indexOf("[updater-linux-proof] proving updated app still launches");
  const lastCleanupBeforeWorkspace = scriptText.lastIndexOf("stop_proof_daemons", workspaceProofIndex);
  assert.ok(workspaceProofIndex > 0, "workspace proof phase should stay present");
  assert.ok(lastCleanupBeforeWorkspace > 0, "workspace proof should clean updater daemons first");
});

test("Linux updater proof runs final workspace flow in a separate home", () => {
  assert.match(scriptText, /workspace_home_dir=""/);
  assert.match(scriptText, /workspace_home_dir="\$\{ARTIFACT_DIR\}\/workspace-home"/);
  assert.match(scriptText, /stop_proof_daemons\(\) \{\s+local proof_home="\$\{1:-\$\{home_dir:-\}\}"/);
  assert.match(scriptText, /stop_proof_daemons "\$\{workspace_home_dir\}"/);
  assert.ok(scriptText.includes('HOME="${workspace_home_dir}" \\'));
  assert.ok(scriptText.includes('XDG_DATA_HOME="${workspace_home_dir}/.local/share" \\'));
  assert.ok(scriptText.includes('XDG_CONFIG_HOME="${workspace_home_dir}/.config" \\'));
  assert.ok(scriptText.includes('XDG_CACHE_HOME="${workspace_home_dir}/.cache" \\'));
  assert.ok(scriptText.includes('CTX_AUTOMATION_SHIPPED_APP_DAEMON_DATA_DIR="${workspace_home_dir}/.ctx" \\'));
  assert.match(
    scriptText,
    /if \[\[ -n "\$\{workspace_home_dir:-\}" \]\]; then\s+stop_proof_daemons "\$\{workspace_home_dir\}" \|\| true\s+fi/,
  );
});

test("Linux updater proof runs the release harness install matrix against updated bundles", () => {
  assert.match(scriptText, /core\/apps\/desktop\/scripts\/run_harness_install_matrix\.sh/);
  assert.match(scriptText, /CTX_HARNESS_INSTALL_MATRIX_DAEMON_BIN="\$\{daemon_bin\}"/);
  assert.match(scriptText, /CTX_HARNESS_INSTALL_MATRIX_BUNDLE_DIR="\$\{bundle_dir\}"/);
  assert.match(scriptText, /--lane release\s+\\\n\s+--platform linux\s+\\\n\s+--target all/);
  assert.doesNotMatch(scriptText, /CTX_UPDATER_RUNTIME_PROVIDER/);
});

test("Linux updater proof exposes explicit phases for split release proof jobs", () => {
  assert.match(scriptText, /PHASES_RAW="\$\{CTX_UPDATER_LINUX_PROOF_PHASES:-all\}"/);
  assert.match(scriptText, /phase_requested\(\) \{/);
  assert.match(scriptText, /updater\|updater-smoke/);
  assert.match(scriptText, /provider-matrix\|linux-provider-matrix/);
  assert.match(scriptText, /clean-workspace\|workspace\|linux-clean-workspace/);
  assert.match(scriptText, /RUN_UPDATER_PHASE=1/);
  assert.match(scriptText, /RUN_PROVIDER_MATRIX_PHASE=1/);
  assert.match(scriptText, /RUN_CLEAN_WORKSPACE_PHASE=1/);
  assert.match(scriptText, /if \[\[ "\$\{RUN_UPDATER_PHASE\}" == "1" \]\]; then/);
  assert.match(scriptText, /if \[\[ "\$\{RUN_PROVIDER_MATRIX_PHASE\}" == "1" \]\]; then/);
  assert.match(scriptText, /if \[\[ "\$\{RUN_CLEAN_WORKSPACE_PHASE\}" == "1" \]\]; then/);
  assert.match(scriptText, /target_channel_not_installed/);
});

