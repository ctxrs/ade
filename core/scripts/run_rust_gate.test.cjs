const assert = require("node:assert/strict");
const test = require("node:test");

const { AGENT_GATE_CRATES, ISOLATED_CARGO_TEST_CRATES } = require("./lib/rust_gate_plan.cjs");
const { MANUAL_ONLY_RUST_CRATES } = require("./lib/rust_workspace_graph.cjs");
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

test("resolveCrates expands reverse deps automatically for ctx-execution-runtime", () => {
  const crates = [
    {
      crateName: "ctx-execution-runtime",
      deps: [],
      reverseDeps: ["ctx-http"],
    },
    {
      crateName: "ctx-http",
      deps: ["ctx-execution-runtime"],
      reverseDeps: ["ctx-mcp"],
    },
    {
      crateName: "ctx-mcp",
      deps: ["ctx-http"],
      reverseDeps: [],
    },
    {
      crateName: "ctx-lsp",
      deps: [],
      reverseDeps: [],
    },
  ];
  const graph = {
    crates,
    cratesByName: new Map(crates.map((crate) => [crate.crateName, crate])),
  };

  assert.deepEqual(
    resolveCrates(graph, {
      agentGate: false,
      all: false,
      changedFiles: [],
      crates: ["ctx-execution-runtime"],
      includeReverseDeps: false,
    }),
    ["ctx-execution-runtime", "ctx-http", "ctx-mcp"],
  );
});

test("resolveCrates runs the full workspace graph for root-level Rust inputs", () => {
  const crates = [
    {
      crateName: "ctx-core",
      relDir: "crates/ctx-core",
      deps: [],
      reverseDeps: ["ctx-http"],
    },
    {
      crateName: "ctx-http",
      relDir: "crates/ctx-http",
      deps: ["ctx-core"],
      reverseDeps: [],
    },
  ];
  const graph = {
    crates,
    cratesByName: new Map(crates.map((crate) => [crate.crateName, crate])),
  };

  assert.deepEqual(
    resolveCrates(graph, {
      agentGate: false,
      all: false,
      changedFiles: ["core/Cargo.toml"],
      crates: [],
      includeReverseDeps: true,
    }),
    ["ctx-core", "ctx-http"],
  );
});

test("resolveCrates excludes manual-only crates from default CI selection", () => {
  assert.equal(MANUAL_ONLY_RUST_CRATES.has("ctx-worker-gateway"), true);

  const crates = [
    {
      crateName: "ctx-core",
      relDir: "crates/ctx-core",
      deps: [],
      reverseDeps: [],
    },
    {
      crateName: "ctx-worker-gateway",
      relDir: "crates/ctx-worker-gateway",
      deps: [],
      reverseDeps: [],
    },
  ];
  const graph = {
    crates,
    cratesByName: new Map(crates.map((crate) => [crate.crateName, crate])),
  };

  assert.deepEqual(
    resolveCrates(graph, {
      agentGate: false,
      all: true,
      changedFiles: [],
      crates: [],
      includeReverseDeps: false,
    }),
    ["ctx-core"],
  );
  assert.deepEqual(
    resolveCrates(graph, {
      agentGate: false,
      all: false,
      changedFiles: ["core/crates/ctx-worker-gateway/src/main.rs"],
      crates: [],
      includeReverseDeps: false,
    }),
    [],
  );
});

test("ctx-http no longer stays on the cargo-isolated test tail", () => {
  assert.equal(ISOLATED_CARGO_TEST_CRATES.has("ctx-http"), false);
});
