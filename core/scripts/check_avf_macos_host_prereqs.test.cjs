const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { spawnSync } = require("node:child_process");

const scriptPath = path.join(__dirname, "check_avf_macos_host_prereqs.sh");

function writeFixture(text) {
  const filePath = path.join(
    fs.mkdtempSync(path.join(os.tmpdir(), "ctx-avf-host-prereqs-")),
    "ioreg-root.txt",
  );
  fs.writeFileSync(filePath, text, "utf8");
  return filePath;
}

function runScript(args, overrideText) {
  const env = {
    ...process.env,
    CTX_AVF_MACOS_IOREG_ROOT_OVERRIDE: writeFixture(overrideText),
  };
  return spawnSync("bash", [scriptPath, ...args], {
    env,
    encoding: "utf8",
  });
}

const unlockedConsole = `
| |   "IOConsoleUsers" = ({"kCGSSessionOnConsoleKey"=Yes,"kCGSessionLoginDoneKey"=Yes,"kCGSSessionUserNameKey"="admin","CGSSessionScreenIsLocked"=No})
`;

const lockedConsole = `
| |   "IOConsoleUsers" = ({"kCGSSessionOnConsoleKey"=Yes,"kCGSessionLoginDoneKey"=Yes,"kCGSSessionUserNameKey"="admin","CGSSessionScreenIsLocked"=Yes})
`;

const noConsoleLogin = `
| |   "IOConsoleUsers" = ()
`;

test("check_avf_macos_host_prereqs skips cleanly when restore smoke is disabled", () => {
  const result = runScript(["--restore-smoke", "skip"], lockedConsole);
  assert.equal(result.status, 0);
  assert.match(result.stdout, /preflight skipped/i);
});

test("check_avf_macos_host_prereqs accepts an unlocked console session for restore smoke", () => {
  const result = runScript(["--restore-smoke", "required"], unlockedConsole);
  assert.equal(result.status, 0);
  assert.match(result.stdout, /logged in and unlocked/i);
});

test("check_avf_macos_host_prereqs rejects a locked console session for restore smoke", () => {
  const result = runScript(["--restore-smoke", "required"], lockedConsole);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /remain unlocked/i);
});

test("check_avf_macos_host_prereqs rejects a missing console login session for restore smoke", () => {
  const result = runScript(["--restore-smoke", "required"], noConsoleLogin);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /interactive macOS console login session/i);
});
