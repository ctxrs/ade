const assert = require("node:assert/strict");
const test = require("node:test");

const { AGENT_GATE_CRATES, ISOLATED_CARGO_TEST_CRATES } = require("./lib/rust_gate_plan.cjs");
const { MANUAL_ONLY_RUST_CRATES } = require("./lib/rust_workspace_graph.cjs");
const {
  applyBazelTestEnv,
  applyDefaultRustGateEnv,
  buildBazelTargetBatches,
  parseArgs,
  resolveCrates,
} = require("./run_rust_gate.cjs");

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

test("applyDefaultRustGateEnv mirrors verify:quick env hygiene without overwriting explicit values", () => {
  const env = {};
  applyDefaultRustGateEnv(env, {
    cargoTestCrates: ["ctx-http"],
    nextestCrates: ["ctx-core"],
  });
  assert.deepEqual(env, {
    CARGO_INCREMENTAL: "0",
    NEXTEST_TEST_THREADS: "2",
    RUST_TEST_THREADS: "1",
  });

  const explicitEnv = {
    CARGO_INCREMENTAL: "1",
    NEXTEST_TEST_THREADS: "9",
    RUST_TEST_THREADS: "7",
  };
  applyDefaultRustGateEnv(explicitEnv, {
    cargoTestCrates: ["ctx-http"],
    nextestCrates: ["ctx-core"],
  });
  assert.deepEqual(explicitEnv, {
    CARGO_INCREMENTAL: "1",
    NEXTEST_TEST_THREADS: "9",
    RUST_TEST_THREADS: "7",
  });
});

test("applyBazelTestEnv caps ctx-http local bazel fanout without overwriting explicit values", () => {
  const env = {};
  applyBazelTestEnv(env, {
    bazelTestCrates: ["ctx-core", "ctx-http"],
  });
  assert.deepEqual(env, {
    CTX_BAZEL_LOCAL_TEST_JOBS: "2",
  });

  const explicitEnv = {
    CTX_BAZEL_LOCAL_TEST_JOBS: "2",
  };
  applyBazelTestEnv(explicitEnv, {
    bazelTestCrates: ["ctx-http"],
  });
  assert.deepEqual(explicitEnv, {
    CTX_BAZEL_LOCAL_TEST_JOBS: "2",
  });
});

test("buildBazelTargetBatches isolates ctx-http suite aliases into sequential bazel phases", () => {
  assert.deepEqual(buildBazelTargetBatches(["ctx-http"]), [
    ["//core/crates/ctx-http:attachments-routing"],
    ["//core/crates/ctx-http:base"],
    ["//core/crates/ctx-http:buffers"],
    ["//core/crates/ctx-http:provider-auth"],
    ["//core/crates/ctx-http:provider-runtime-live"],
    ["//core/crates/ctx-http:provider-runtime-simulated"],
    ["//core/crates/ctx-http:repo-vcs"],
    ["//core/crates/ctx-http:sandbox-runtime-container-e2e"],
    ["//core/crates/ctx-http:sandbox-runtime-memory-leak"],
    ["//core/crates/ctx-http:sandbox-runtime-resource-governance"],
    ["//core/crates/ctx-http:sandbox-runtime-simulated"],
    ["//core/crates/ctx-http:subagents-control"],
    ["//core/crates/ctx-http:subagents-local-runtime"],
    ["//core/crates/ctx-http:turns-terminal"],
    ["//core/crates/ctx-http:updates-release"],
    ["//core/crates/ctx-http:workspace-stream"],
  ]);
});
