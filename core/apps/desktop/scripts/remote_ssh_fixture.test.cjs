const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..", "..", "..");
const scriptPath = path.join(repoRoot, "core/apps/desktop/scripts/remote_ssh_fixture.sh");
const script = fs.readFileSync(scriptPath, "utf8");

test("remote ssh fixture exposes explicit host-mode and release-fixture preseeding hooks", () => {
  assert.match(
    script,
    /remote_ssh_fixture\.sh start \[--runtime auto\|docker\|nerdctl] \[--auth-mode key\|password] \[--password VALUE] \[--state-file PATH] \[--log-dir PATH] \[--user NAME] \[--host-mode fresh-install\|existing-installed-host\|version-mismatch] \[--export-container-lane]/,
  );
  assert.match(script, /LOCAL_RELEASE_FIXTURE_SCRIPT="\$\{SCRIPT_DIR\}\/local_release_fixture\.cjs"/);
  assert.match(script, /HOST_MODE="\$\{CTX_AUTOMATION_REMOTE_FIXTURE_HOST_MODE:-fresh-install\}"/);
  assert.match(script, /CTX_AUTOMATION_REMOTE_RELEASE_FIXTURE_STATE_FILE/);
  assert.match(
    script,
    /node "\$\{LOCAL_RELEASE_FIXTURE_SCRIPT\}" print-artifact --state-file "\$\{RELEASE_FIXTURE_STATE_FILE\}" --arch "\$\{remote_arch\}"/,
  );
  assert.match(
    script,
    /node "\$\{LOCAL_RELEASE_FIXTURE_SCRIPT\}" print-expected-version --state-file "\$\{RELEASE_FIXTURE_STATE_FILE\}"/,
  );
  assert.match(script, /quote_export CTX_AUTOMATION_REMOTE_FIXTURE_HOST_MODE/);
  assert.match(script, /quote_export CTX_AUTOMATION_REMOTE_EXPECTED_MANAGED_VERSION/);
});

test("remote ssh fixture version-mismatch mode installs a runnable wrapper that can self-replace", () => {
  assert.match(script, /remote_real_bin="\$\{remote_bin_dir\}\/ctx\.expected"/);
  assert.match(script, /case "\$\{1:-\}" in[\s\S]*--version[\s\S]*version[\s\S]*self-update/s);
  assert.match(script, /cp "\$\{REAL_BIN\}" "\$\{SELF_PATH\}"/);
  assert.match(script, /exec "\$\{SELF_PATH\}" "\$@"/);
  assert.match(script, /exec "\$\{REAL_BIN\}" "\$@"/);
});
