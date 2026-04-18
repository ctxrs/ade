const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { spawnSync } = require("node:child_process");

const repoRoot = path.resolve(__dirname, "..", "..");
const coreRoot = path.join(repoRoot, "core");
const scriptPath = path.join(__dirname, "affected_tests.sh");

function runScenario(changedFiles, options = {}) {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "affected-tests-contract-"));
  const commandLogPath = path.join(tempRoot, "commands.log");

  const result = spawnSync("bash", [scriptPath], {
    cwd: coreRoot,
    env: {
      ...process.env,
      ...options.env,
      CTX_AFFECTED_TESTS_CHANGED_FILES: changedFiles.join("\n"),
      CTX_AFFECTED_TESTS_COMMAND_LOG: commandLogPath,
      CTX_AFFECTED_TESTS_SKIP_CACHE_ENV: "1",
      CTX_AFFECTED_TESTS_UNAME: options.unameValue || "Darwin",
      COMMAND_LOG_PATH: commandLogPath,
      NODE_OPTIONS: "",
    },
    encoding: "utf8",
    stdio: "pipe",
    timeout: 180_000,
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
    "bash -lc pnpm rust:turbo:check",
    "bash -lc pnpm exec node scripts/run_rust_gate.cjs --mode workspace --include-reverse-deps --clippy --test-strategy mixed --changed-file core/crates/ctx-provider-accounts/src/lib.rs",
  ]);
});

test("ctx-http changes fan out into suite-level commands", () => {
  const commands = runScenario(["core/crates/ctx-http/src/api/mod.rs"]);

  assert.deepEqual(commands, [
    "bash -lc node scripts/ctx_http_suite_task.cjs --suite attachments-routing",
    "bash -lc node scripts/ctx_http_suite_task.cjs --suite lsp",
    "bash -lc node scripts/ctx_http_suite_task.cjs --suite provider-auth",
    "bash -lc node scripts/ctx_http_suite_task.cjs --suite provider-runtime-simulated",
    "bash -lc node scripts/ctx_http_suite_task.cjs --suite repo-vcs",
    "bash -lc node scripts/ctx_http_suite_task.cjs --suite sandbox-runtime-simulated",
    "bash -lc node scripts/ctx_http_suite_task.cjs --suite subagents-control",
    "bash -lc node scripts/ctx_http_suite_task.cjs --suite turns-terminal",
    "bash -lc node scripts/ctx_http_suite_task.cjs --suite updates-release",
    "bash -lc node scripts/ctx_http_suite_task.cjs --suite workspace-stream",
  ]);
});

test("ctx-providers changes use the targeted Rust fallback", () => {
  const commands = runScenario(["core/crates/ctx-providers/src/lib.rs"]);

  assert.deepEqual(commands, [
    "bash -lc pnpm rust:turbo:check",
    "bash -lc pnpm exec node scripts/run_rust_gate.cjs --mode workspace --include-reverse-deps --clippy --test-strategy mixed --changed-file core/crates/ctx-providers/src/lib.rs",
  ]);
});

test("web high-risk changes still trigger the safety fallback", () => {
  const commands = runScenario(["core/apps/web/src/state/providerOnboardingCoordinator.ts"]);

  assert.deepEqual(commands, ["bash -lc pnpm bazel:web:test", "bash -lc pnpm verify:e2e"]);
});

test("root-level Rust config changes still trigger the Rust gate", () => {
  const commands = runScenario(["core/Cargo.lock"]);

  assert.deepEqual(commands, [
    "bash -lc pnpm rust:turbo:check",
    "bash -lc pnpm exec node scripts/run_rust_gate.cjs --mode workspace --include-reverse-deps --clippy --test-strategy mixed --changed-file core/Cargo.lock",
  ]);
});

test("root-level Rust toolchain change still triggers the Rust gate", () => {
  const commands = runScenario(["core/rust-toolchain.toml"]);

  assert.deepEqual(commands, [
    "bash -lc pnpm rust:turbo:check",
    "bash -lc pnpm exec node scripts/run_rust_gate.cjs --mode workspace --include-reverse-deps --clippy --test-strategy mixed --changed-file core/rust-toolchain.toml",
  ]);
});

test("combined root-level Rust and crate changes still pass both changed files through", () => {
  const commands = runScenario([
    "core/Cargo.toml",
    "core/crates/ctx-provider-accounts/src/lib.rs",
  ]);

  assert.deepEqual(commands, [
    "bash -lc pnpm rust:turbo:check",
    "bash -lc pnpm exec node scripts/run_rust_gate.cjs --mode workspace --include-reverse-deps --clippy --test-strategy mixed --changed-file core/Cargo.toml --changed-file core/crates/ctx-provider-accounts/src/lib.rs",
  ]);
});

test("no-change path uses the platform-aware fast gate", () => {
  const commands = runScenario([], { unameValue: "Linux" });

  assert.deepEqual(commands, [
    "pnpm test:agent:linux-rbe",
  ]);
});
