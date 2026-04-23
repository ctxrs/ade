const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const { shellQuote, formatRemoteSshError } = require("./specs/helpers/remote_updater_proof.cjs");

const HELPER_PATH = path.join(__dirname, "specs", "helpers", "remote_updater_proof.cjs");
const SPEC_PATH = path.join(__dirname, "specs", "updater-remote-daemon-e2e.spec.cjs");

test("remote updater proof shell quoting prevents remote pre-expansion", () => {
  const quoted = shellQuote('mkdir -p "$proof_root"\nprintf %s "$tmp_dir"\n');
  assert.equal(quoted, `'mkdir -p "$proof_root"\nprintf %s "$tmp_dir"\n'`);
});

test("remote updater proof shell quoting preserves embedded single quotes", () => {
  const quoted = shellQuote("printf '%s\\n' \"$value\"");
  assert.equal(quoted, `'printf '\\''%s\\n'\\'' "$value"'`);
});

test("remote updater proof ssh errors include stderr and stdout", () => {
  const error = new Error("ssh exited with status 1");
  error.status = 1;
  error.stderr = "curl failed";
  error.stdout = "remote diagnostics";
  assert.equal(formatRemoteSshError(error), "curl failed\nremote diagnostics\nssh exited unsuccessfully (status=1)");
});

test("remote updater proof ssh errors do not echo command text", () => {
  const error = new Error("Command failed: ssh host curl -H 'authorization: Bearer secret-token'");
  error.status = 255;
  assert.equal(formatRemoteSshError(error), "ssh exited unsuccessfully (status=255)");
});

test("remote updater proof download helper leaves path variables for the remote shell", () => {
  const source = fs.readFileSync(HELPER_PATH, "utf8");
  assert.doesNotMatch(source, /\$\{dest\}/);
  assert.match(source, /curl --fail --show-error --location[\s\S]*-o "\$dest\.partial" "\$url"/);
  assert.match(source, /mv -f "\$dest\.partial" "\$dest"/);
});

test("remote updater proof sends remote scripts over stdin instead of argv", () => {
  const source = fs.readFileSync(HELPER_PATH, "utf8");
  assert.match(source, /remoteSsh\("bash -s", \{ \.\.\.options, input: script \}\)/);
  assert.doesNotMatch(source, /bash -lc \$\{shellQuote\(script\)\}/);
});

test("remote updater proof keeps HTTP secrets out of curl argv", () => {
  const source = fs.readFileSync(HELPER_PATH, "utf8");
  assert.match(source, /tmp_curl_config="\$\(mktemp\)"/);
  assert.match(source, /status="\$\(curl --config "\$tmp_curl_config"\)"/);
  assert.doesNotMatch(source, /-H 'authorization: Bearer \$\{authToken\}'/);
  assert.doesNotMatch(source, /--data \$\{JSON\.stringify\(bodyJson\)\}/);
});

test("remote updater proof normalizes home-relative remote ctx paths", () => {
  const source = fs.readFileSync(HELPER_PATH, "utf8");
  assert.match(source, /const REMOTE_CTX_BIN = String\([^]*"\$HOME\/\.ctx\/bin\/ctx"\)/);
  assert.match(source, /normalize_remote_path\(\)/);
  assert.match(source, /ctx_bin="\$\(normalize_remote_path \$\{shellQuote\(REMOTE_CTX_BIN\)\}\)"/);
});

test("remote updater proof requires version changes and checks busy-turn outcomes", () => {
  const source = fs.readFileSync(SPEC_PATH, "utf8");
  assert.match(source, /process\.env\.CTX_UPDATER_E2E_EXPECT_VERSION_CHANGE,\s*\n\s*true,/);
  assert.match(source, /assertTurnStatus\(terminalTurn, \["completed"\], "pending restart-on-idle"\)/);
  assert.match(source, /assertTurnStatus\(terminalTurn, \["failed", "cancelled"\], "pending restart-now"\)/);
  assert.match(source, /assertTurnStatus\(terminalTurn, \["failed", "cancelled"\], "incompatible reconnect"\)/);
});
