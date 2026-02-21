import test from "node:test";
import assert from "node:assert/strict";
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
  const args = buildPiLaunchArgs({
    PATH: "",
  });

  assert.match(args.command, /node_modules[\/\\]\.bin[\/\\]pi(\.cmd)?$/i);
  assert.deepEqual(args.args, [
    "--mode",
    "rpc",
    "--no-session",
  ]);
});
