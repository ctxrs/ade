const assert = require("node:assert/strict");
const test = require("node:test");

const { AGENT_GATE_CRATES } = require("./lib/rust_gate_plan.cjs");
const { parseArgs, resolveCrates } = require("./run_rust_gate.cjs");

test("parseArgs accepts --agent-gate without additional selection flags", () => {
  assert.deepEqual(parseArgs(["--agent-gate", "--test-strategy", "mixed"]), {
    agentGate: true,
    all: false,
    changedFiles: [],
    crates: [],
    includeReverseDeps: false,
    mode: "workspace",
    runClippy: false,
    testStrategy: "mixed",
  });
});

test("parseArgs rejects mixing --agent-gate with other selectors", () => {
  assert.throws(
    () => parseArgs(["--agent-gate", "--crate", "ctx-http"]),
    /--agent-gate cannot be combined/,
  );
});

test("resolveCrates returns the curated agent gate crate list for --agent-gate", () => {
  assert.deepEqual(
    resolveCrates({ crates: [{ crateName: "ctx-http" }] }, { agentGate: true }),
    AGENT_GATE_CRATES,
  );
});
