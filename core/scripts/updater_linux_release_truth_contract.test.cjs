const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const repoRoot = path.resolve(__dirname, "..", "..");
const scriptPath = path.join(repoRoot, "scripts", "tests", "updater_linux_release_truth.sh");
const scriptText = fs.readFileSync(scriptPath, "utf8");

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
