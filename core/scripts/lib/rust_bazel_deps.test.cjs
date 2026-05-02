const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");

const config = require("../rust_bazel_deps.config.cjs");
const {
  buildRealWorkspaceModel,
  buildRustBazelDeps,
  parseModuleCrateSpecs,
  renderRustBazelDepsStarlark,
  syncGeneratedRustBazelDeps,
} = require("./rust_bazel_deps.cjs");

const coreRoot = path.resolve(__dirname, "../..");
const repoRoot = path.resolve(coreRoot, "..");

function fixtureConfig() {
  return {
    version: 1,
    generatedOutputPath: "tools/bazel/rust_deps.generated.bzl",
    generatedDepConsumerCrates: ["app-crate"],
    manualCrates: {
      "manual-crate": {
        owner: "test",
        rationale: "fixture manual crate",
      },
    },
    procMacroDeps: {
      "async-trait": {
        owner: "test",
        rationale: "fixture proc macro dep",
      },
    },
  };
}

function fixtureMetadata() {
  const root = "/repo/core";
  return {
    packages: [
      {
        dependencies: [
          dep("anyhow"),
          dep("async-trait"),
          dep("tempfile", { kind: "dev" }),
          dep("cc", { kind: "build" }),
          dep("optional-crate", { optional: true }),
          dep("libc", { target: "cfg(unix)" }),
          dep("regex"),
          pathDep("target-util-crate", `${root}/crates/target-util-crate`, { target: "cfg(windows)" }),
          pathDep("util-crate", `${root}/crates/util-crate`),
        ],
        id: "path+file:///repo/core/crates/app-crate#0.1.0",
        manifest_path: `${root}/crates/app-crate/Cargo.toml`,
        name: "app-crate",
      },
      {
        dependencies: [],
        id: "path+file:///repo/core/crates/util-crate#0.1.0",
        manifest_path: `${root}/crates/util-crate/Cargo.toml`,
        name: "util-crate",
      },
      {
        dependencies: [dep("anyhow")],
        id: "path+file:///repo/core/crates/manual-crate#0.1.0",
        manifest_path: `${root}/crates/manual-crate/Cargo.toml`,
        name: "manual-crate",
      },
    ],
    workspace_members: [
      "path+file:///repo/core/crates/app-crate#0.1.0",
      "path+file:///repo/core/crates/util-crate#0.1.0",
      "path+file:///repo/core/crates/manual-crate#0.1.0",
    ],
  };
}

function dep(name, overrides = {}) {
  return {
    features: [],
    kind: null,
    name,
    optional: false,
    path: undefined,
    rename: null,
    source: "registry+https://github.com/rust-lang/crates.io-index",
    target: null,
    ...overrides,
  };
}

function pathDep(name, depPath, overrides = {}) {
  return {
    ...dep(name, {
      path: depPath,
      source: null,
    }),
    ...overrides,
  };
}

function moduleText(...packages) {
  return packages.map((pkg) => `crate.spec(package = "${pkg}", version = "1")`).join("\n");
}

function fixtureFileExists(filePath) {
  return /\/core\/crates\/(app-crate|util-crate)\/BUILD\.bazel$/.test(filePath);
}

function fixtureReadFile(filePath) {
  if (/\/core\/crates\/(app-crate|util-crate)\/BUILD\.bazel$/.test(filePath)) {
    return 'rust_library(\n    name = "lib",\n)\n';
  }
  throw new Error(`unexpected fixture read: ${filePath}`);
}

test("generator maps cargo deps into deterministic Bazel dep buckets", () => {
  const model = buildRustBazelDeps({
    config: fixtureConfig(),
    coreRoot: "/repo/core",
    fileExists: fixtureFileExists,
    metadata: fixtureMetadata(),
    moduleBazelText: moduleText("anyhow", "async-trait", "cc", "regex", "tempfile"),
    readFile: fixtureReadFile,
    repoRoot: "/repo",
  });

  assert.deepEqual(model.entries["app-crate"], {
    build_deps: ["@crates//:cc"],
    deps: [
      "//core/crates/util-crate:lib",
      "@crates//:anyhow",
      "@crates//:regex",
    ],
    dev_deps: ["@crates//:tempfile"],
    dev_proc_macro_deps: [],
    proc_macro_deps: ["@crates//:async-trait"],
  });
  assert.equal(model.entries["manual-crate"], undefined);
  assert.equal(model.entries["util-crate"], undefined);
  assert.equal(model.skippedCrates["manual-crate"], "fixture manual crate");
});

test("generator validates external deps against MODULE.bazel crate specs", () => {
  assert.throws(
    () => buildRustBazelDeps({
      config: fixtureConfig(),
      coreRoot: "/repo/core",
      fileExists: fixtureFileExists,
      metadata: fixtureMetadata(),
      moduleBazelText: moduleText("async-trait", "cc", "regex", "tempfile"),
      readFile: fixtureReadFile,
      repoRoot: "/repo",
    }),
    /Cargo dependency 'anyhow' is not declared in MODULE\.bazel/,
  );
});

test("generator rejects renamed Cargo deps until alias support exists", () => {
  const metadata = fixtureMetadata();
  metadata.packages[0].dependencies.push(dep("renamed-package", { rename: "renamed_dep" }));

  assert.throws(
    () => buildRustBazelDeps({
      config: fixtureConfig(),
      coreRoot: "/repo/core",
      fileExists: fixtureFileExists,
      metadata,
      moduleBazelText: moduleText("anyhow", "async-trait", "cc", "regex", "renamed-package", "tempfile"),
      readFile: fixtureReadFile,
      repoRoot: "/repo",
    }),
    /renames package 'renamed-package'/,
  );
});

test("generator validates workspace path deps expose a lib target", () => {
  assert.throws(
    () => buildRustBazelDeps({
      config: fixtureConfig(),
      coreRoot: "/repo/core",
      fileExists: fixtureFileExists,
      metadata: fixtureMetadata(),
      moduleBazelText: moduleText("anyhow", "async-trait", "cc", "regex", "tempfile"),
      readFile: (filePath) => {
        if (filePath.endsWith("/util-crate/BUILD.bazel")) {
          return 'rust_binary(\n    name = "tool",\n)\n';
        }
        return fixtureReadFile(filePath);
      },
      repoRoot: "/repo",
    }),
    /must expose a Bazel target named 'lib'/,
  );
});

test("rendered Starlark is deterministic and timestamp-free", () => {
  const rendered = renderRustBazelDepsStarlark({
    entries: {
      zed: {
        build_deps: [],
        deps: ["@crates//:zed"],
        dev_deps: [],
        dev_proc_macro_deps: [],
        proc_macro_deps: [],
      },
      alpha: {
        build_deps: [],
        deps: ["@crates//:alpha"],
        dev_deps: ["@crates//:tempfile"],
        dev_proc_macro_deps: [],
        proc_macro_deps: ["@crates//:async-trait"],
      },
    },
  });

  assert.equal(rendered.includes("2026"), false);
  assert.match(rendered, /^# @generated/m);
  assert.ok(rendered.indexOf('"alpha"') < rendered.indexOf('"zed"'));
});

test("check mode fails when generated output is stale", () => {
  assert.throws(
    () => syncGeneratedRustBazelDeps({
      check: true,
      config: fixtureConfig(),
      coreRoot: "/repo/core",
      fileExists: fixtureFileExists,
      metadata: fixtureMetadata(),
      moduleBazelText: moduleText("anyhow", "async-trait", "cc", "regex", "tempfile"),
      outPath: "/repo/tools/bazel/rust_deps.generated.bzl",
      readFile: (filePath) => {
        if (filePath.endsWith("BUILD.bazel")) {
          return fixtureReadFile(filePath);
        }
        return "stale\n";
      },
      repoRoot: "/repo",
    }),
    /generated Rust Bazel deps are stale/,
  );
});

test("MODULE parser extracts crate specs", () => {
  assert.deepEqual(
    [...parseModuleCrateSpecs('crate.spec(package = "serde", version = "1")\ncrate.spec(package = "serde_json", version = "1")')].sort(),
    ["serde", "serde_json"],
  );
});

test("real workspace model covers migrated crates and skips manual crates", () => {
  const { model } = buildRealWorkspaceModel({ config, coreRoot, repoRoot });

  assert.deepEqual(model.entries["ctx-bundled-assets"], {
    build_deps: [],
    deps: [
      "@crates//:serde",
      "@crates//:serde_json",
      "@crates//:tracing",
    ],
    dev_deps: ["@crates//:tempfile"],
    dev_proc_macro_deps: [],
    proc_macro_deps: [],
  });
  assert.deepEqual(model.entries["ctx-core"].deps, [
    "@crates//:chrono",
    "@crates//:serde",
    "@crates//:serde_json",
    "@crates//:uuid",
  ]);
  assert.deepEqual(model.entries["ctx-provider-matrix"], {
    build_deps: [],
    deps: [
      "//core/crates/ctx-provider-accounts:lib",
      "@crates//:anyhow",
      "@crates//:semver",
      "@crates//:serde",
      "@crates//:serde_json",
      "@crates//:tokio",
    ],
    dev_deps: ["@crates//:tempfile"],
    dev_proc_macro_deps: [],
    proc_macro_deps: [],
  });
  assert.deepEqual(model.entries["ctx-llm-relay-contract"].deps, [
    "@crates//:chrono",
    "@crates//:serde",
    "@crates//:serde_json",
    "@crates//:thiserror",
  ]);
  assert.deepEqual(model.entries["ctx-llm-relay-authority"].deps, [
    "//core/crates/ctx-llm-relay-contract:lib",
    "@crates//:anyhow",
    "@crates//:axum",
    "@crates//:base64",
    "@crates//:chrono",
    "@crates//:clap",
    "@crates//:jsonwebtoken",
    "@crates//:ring",
    "@crates//:serde",
    "@crates//:serde_json",
    "@crates//:sqlx",
    "@crates//:thiserror",
    "@crates//:tokio",
    "@crates//:tracing",
    "@crates//:tracing-subscriber",
    "@crates//:uuid",
  ]);
  assert.deepEqual(model.entries["ctx-llm-relay-authority"].dev_deps, [
    "@crates//:tower",
  ]);
  assert.deepEqual(model.entries["ctx-llm-relay-authority"].proc_macro_deps, [
    "@crates//:async-trait",
  ]);
  assert.deepEqual(model.entries["ctx-sandbox-contract"].dev_deps, [
    "@crates//:chrono",
    "@crates//:uuid",
  ]);
  assert.equal(model.entries["ctx-http"], undefined);
  assert.equal(model.entries["ctx-load-test"], undefined);
  assert.ok(model.entries["ctx-runtime-assets"].deps.includes("//core/crates/ctx-harness-setup:lib"));
});
