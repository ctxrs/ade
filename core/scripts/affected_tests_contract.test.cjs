const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { execFileSync } = require("node:child_process");

const repoRoot = path.resolve(__dirname, "..", "..");
const coreRoot = path.join(repoRoot, "core");
const scriptPath = path.join(__dirname, "affected_tests.sh");

function writeExecutable(filePath, contents) {
  fs.writeFileSync(filePath, contents, { mode: 0o755 });
}

function runScenario(changedFiles) {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "affected-tests-contract-"));
  const binDir = path.join(tempRoot, "bin");
  const commandLogPath = path.join(tempRoot, "commands.log");
  fs.mkdirSync(binDir, { recursive: true });

  writeExecutable(
    path.join(binDir, "git"),
    `#!/usr/bin/env bash
set -euo pipefail
args=("$@")
if [[ "\${args[0]:-}" == "-C" ]]; then
  args=("\${args[@]:2}")
fi
case "\${args[0]:-}" in
  diff)
    printf '%s\n' "\${STUB_CHANGED_FILES}"
    ;;
  merge-base)
    printf 'stub-merge-base\n'
    ;;
  *)
    printf 'unexpected git invocation: %s\n' "$*" >&2
    exit 1
    ;;
esac
`,
  );

  writeExecutable(
    path.join(binDir, "cargo"),
    `#!/usr/bin/env bash
set -euo pipefail
printf 'cargo %s\n' "$*" >> "\${COMMAND_LOG_PATH}"
`,
  );

  writeExecutable(
    path.join(binDir, "pnpm"),
    `#!/usr/bin/env bash
set -euo pipefail
printf 'pnpm %s\n' "$*" >> "\${COMMAND_LOG_PATH}"
`,
  );

  try {
    execFileSync("bash", [scriptPath], {
      cwd: coreRoot,
      env: {
        ...process.env,
        COMMAND_LOG_PATH: commandLogPath,
        PATH: `${binDir}:${process.env.PATH}`,
        STUB_CHANGED_FILES: changedFiles.join("\n"),
      },
      stdio: "pipe",
    });
  } catch (error) {
    const stdout = error.stdout ? error.stdout.toString("utf8") : "";
    const stderr = error.stderr ? error.stderr.toString("utf8") : "";
    throw new Error(`affected_tests.sh failed\nstdout:\n${stdout}\nstderr:\n${stderr}`);
  }

  const commandLog = fs.existsSync(commandLogPath)
    ? fs.readFileSync(commandLogPath, "utf8")
    : "";
  fs.rmSync(tempRoot, { recursive: true, force: true });
  return commandLog.trim().split("\n").filter(Boolean);
}

test("leaf Rust crate changes stay on the targeted fast path", () => {
  const commands = runScenario(["core/crates/ctx-provider-accounts/src/lib.rs"]);

  assert.deepEqual(commands, ["cargo test -q -p ctx-provider-accounts"]);
});

test("ctx-http changes still trigger the high-risk fallback", () => {
  const commands = runScenario(["core/crates/ctx-http/src/api/mod.rs"]);

  assert.deepEqual(commands, ["pnpm test:agent", "pnpm verify:e2e"]);
});
