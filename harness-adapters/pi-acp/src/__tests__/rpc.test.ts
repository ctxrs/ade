import test from "node:test";
import assert from "node:assert/strict";
import { chmodSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { buildPiLaunchArgs, waitForIdleState } from "../rpc.js";

test("buildPiLaunchArgs includes provider/model and extra args", () => {
  const args = buildPiLaunchArgs({
    PI_ACP_PI_COMMAND: "pi-bin",
    PI_ACP_PROVIDER: "openrouter",
    PI_ACP_MODEL: "openai/gpt-5",
    PI_ACP_PI_ARGS: "--foo bar",
  });

  assert.equal(args.command, "pi-bin");
  assert.deepEqual(args.args, [
    "--mode",
    "rpc",
    "--no-session",
    "--provider",
    "openrouter",
    "--model",
    "openai/gpt-5",
    "--foo",
    "bar",
  ]);
});

test("waitForIdleState settles quickly when state is already idle", async () => {
  let polls = 0;
  await waitForIdleState(
    async () => {
      polls += 1;
      return { isStreaming: false, isCompacting: false, pendingMessageCount: 0 };
    },
    { timeoutMs: 200, pollIntervalMs: 0, idlePollThreshold: 2 },
  );
  assert.equal(polls, 2);
});

test("waitForIdleState waits for busy to idle transition", async () => {
  const states = [
    { isStreaming: true, isCompacting: false, pendingMessageCount: 1 },
    { isStreaming: false, isCompacting: false, pendingMessageCount: 0 },
  ];

  await waitForIdleState(
    async () => {
      const next = states.shift();
      assert.ok(next);
      return next;
    },
    { timeoutMs: 200, pollIntervalMs: 0 },
  );
});

test("buildPiLaunchArgs prefers bundled local pi when available", () => {
  const root = mkdtempTree();
  const binDir = join(root, "../node_modules/.bin");
  mkdirSync(binDir, { recursive: true });
  const piPath = join(binDir, process.platform === "win32" ? "pi.cmd" : "pi");
  writeFileSync(piPath, "#!/usr/bin/env node\n");
  chmodSync(piPath, 0o755);

  const args = buildPiLaunchArgs(
    {
      PATH: "",
    },
    root,
  );

  assert.match(args.command, /node_modules[\/\\]\.bin[\/\\]pi(\.cmd)?$/i);
  assert.deepEqual(args.args, [
    "--mode",
    "rpc",
    "--no-session",
  ]);
});

test("buildPiLaunchArgs falls back to bundled package CLI when .bin symlink is absent", () => {
  const root = mkdtempTree();
  const cliPath = join(
    root,
    "../node_modules/@mariozechner/pi-coding-agent/dist/cli.js",
  );
  mkdirSync(join(cliPath, ".."), { recursive: true });
  writeFileSync(cliPath, "console.log('pi');\n");

  const args = buildPiLaunchArgs(
    {
      PATH: "",
      PI_ACP_PROVIDER: "openrouter",
      PI_ACP_MODEL: "google/gemini-3-flash-preview",
    },
    root,
  );

  assert.equal(args.command, process.execPath);
  assert.deepEqual(args.args, [
    cliPath,
    "--mode",
    "rpc",
    "--no-session",
    "--provider",
    "openrouter",
    "--model",
    "google/gemini-3-flash-preview",
  ]);
});

function mkdtempTree(): string {
  const root = join(
    tmpdir(),
    `pi-acp-rpc-test-${process.pid}-${Date.now()}-${Math.random().toString(16).slice(2)}`,
    "dist",
  );
  mkdirSync(root, { recursive: true });
  return root;
}
