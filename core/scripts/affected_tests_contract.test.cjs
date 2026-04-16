const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { spawnSync } = require("node:child_process");

const repoRoot = path.resolve(__dirname, "..", "..");
const coreRoot = path.join(repoRoot, "core");
const scriptPath = path.join(__dirname, "affected_tests.sh");

function writeExecutable(filePath, contents) {
  fs.writeFileSync(filePath, contents, { mode: 0o755 });
}

function runScenario(changedFiles, options = {}) {
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

  writeExecutable(
    path.join(binDir, "node"),
    `#!/usr/bin/env bash
set -euo pipefail
if [[ "\${1:-}" == "scripts/print_ctx_cache_env.cjs" ]]; then
  exit 0
fi
printf 'unexpected node invocation: %s\n' "$*" >&2
exit 1
`,
  );

  writeExecutable(
    path.join(binDir, "uname"),
    `#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "\${STUB_UNAME:-Darwin}"
`,
  );

  const result = spawnSync("bash", [scriptPath], {
    cwd: coreRoot,
    env: {
      ...process.env,
      ...options.env,
      COMMAND_LOG_PATH: commandLogPath,
      NODE_OPTIONS: "",
      PATH: `${binDir}:${process.env.PATH}`,
      STUB_CHANGED_FILES: changedFiles.join("\n"),
      STUB_UNAME: options.unameValue || "Darwin",
    },
    encoding: "utf8",
    stdio: "pipe",
    timeout: 60_000,
  });
  if (result.error || result.status !== 0) {
    const stdout = result.stdout || "";
    const stderr = result.stderr || "";
    const detail = result.error ? `error: ${result.error.message}\n` : "";
    throw new Error(`affected_tests.sh failed\n${detail}stdout:\n${stdout}\nstderr:\n${stderr}`);
  }

  const commandLog = fs.existsSync(commandLogPath)
    ? fs.readFileSync(commandLogPath, "utf8")
    : "";
  fs.rmSync(tempRoot, { recursive: true, force: true });
  return commandLog.trim().split("\n").filter(Boolean);
}

test("leaf Rust crate changes stay on the targeted fast path", () => {
  const commands = runScenario(["core/crates/ctx-provider-accounts/src/lib.rs"]);

  assert.deepEqual(commands, [
    "pnpm rust:turbo:check",
    "pnpm exec node scripts/run_rust_gate.cjs --mode workspace --include-reverse-deps --clippy --test-strategy mixed --changed-file core/crates/ctx-provider-accounts/src/lib.rs",
  ]);
});

test("ctx-http changes trigger the full safety fallback", () => {
  const commands = runScenario(["core/crates/ctx-http/src/api/mod.rs"]);

  assert.deepEqual(commands, ["pnpm test:agent", "pnpm verify:e2e"]);
});

test("ctx-providers changes trigger the full safety fallback", () => {
  const commands = runScenario(["core/crates/ctx-providers/src/lib.rs"]);

  assert.deepEqual(commands, ["pnpm test:agent", "pnpm verify:e2e"]);
});

test("web high-risk changes still trigger the safety fallback", () => {
  const commands = runScenario(["core/apps/web/src/state/providerOnboardingCoordinator.ts"]);

  assert.deepEqual(commands, ["pnpm test:agent", "pnpm verify:e2e"]);
});

test("linux fast-gate defaults to the linux-rbe agent gate", () => {
  const commands = runScenario(["core/crates/ctx-http/src/api/mod.rs"], {
    unameValue: "Linux",
  });

  assert.deepEqual(commands, ["pnpm test:agent:linux-rbe", "pnpm verify:e2e"]);
});

test("explicit fast-gate override forces the linux-rbe fallback", () => {
  const commands = runScenario(["core/crates/ctx-http/src/api/mod.rs"], {
    env: {
      CTX_AFFECTED_TESTS_FAST_GATE: "test:agent:linux-rbe",
    },
  });

  assert.deepEqual(commands, ["pnpm test:agent:linux-rbe", "pnpm verify:e2e"]);
});

test("root-level Rust config changes still trigger the Rust gate", () => {
  const commands = runScenario(["core/Cargo.lock"]);

  assert.deepEqual(commands, [
    "pnpm rust:turbo:check",
    "pnpm exec node scripts/run_rust_gate.cjs --mode workspace --include-reverse-deps --clippy --test-strategy mixed --changed-file core/Cargo.lock",
  ]);
});

test("root-level Rust toolchain change still triggers the Rust gate", () => {
  const commands = runScenario(["core/rust-toolchain.toml"]);

  assert.deepEqual(commands, [
    "pnpm rust:turbo:check",
    "pnpm exec node scripts/run_rust_gate.cjs --mode workspace --include-reverse-deps --clippy --test-strategy mixed --changed-file core/rust-toolchain.toml",
  ]);
});

test("combined root-level Rust and crate changes still pass both changed files through", () => {
  const commands = runScenario([
    "core/Cargo.toml",
    "core/crates/ctx-provider-accounts/src/lib.rs",
  ]);

  assert.deepEqual(commands, [
    "pnpm rust:turbo:check",
    "pnpm exec node scripts/run_rust_gate.cjs --mode workspace --include-reverse-deps --clippy --test-strategy mixed --changed-file core/Cargo.toml --changed-file core/crates/ctx-provider-accounts/src/lib.rs",
  ]);
});
