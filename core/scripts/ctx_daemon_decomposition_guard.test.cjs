const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  COLLAPSED_PATHS,
  RATCHETED_FILE_LIMITS,
  PACKAGE_SHAPE_BOUNDARY_CRATES,
  PACKAGE_SHAPE_FORBIDDEN_BACKEDGE_DEPS,
  checkCargoDependencyDirection,
  checkCollapsedPaths,
  checkHeadProjectionPurity,
  checkRatchetedFileCaps,
  countLines,
  evaluateDecompositionBoundaries,
  isPackageShapeBoundaryCrate,
  isServiceOrRuntimeCrate,
  isWorkspaceActiveSnapshotForbiddenDependency,
  packageNameFromCargoToml,
  parseCargoDependencies,
  run,
} = require("./ctx_daemon_decomposition_guard.cjs");

const makeRoot = () => fs.mkdtempSync(path.join(os.tmpdir(), "ctx-daemon-decomposition-"));

const writeFile = (rootDir, relativePath, contents) => {
  const absolutePath = path.join(rootDir, relativePath);
  fs.mkdirSync(path.dirname(absolutePath), { recursive: true });
  fs.writeFileSync(absolutePath, contents, "utf8");
};

const lines = (lineCount) =>
  `${Array.from({ length: lineCount }, (_, index) => `line ${index + 1}`).join("\n")}\n`;

test("countLines handles newline-terminated and unterminated files", () => {
  assert.equal(countLines("a\nb\n"), 2);
  assert.equal(countLines("a\nb"), 2);
  assert.equal(countLines(""), 0);
});

test("collapsed god-file paths are rejected when recreated", () => {
  const rootDir = makeRoot();
  writeFile(rootDir, COLLAPSED_PATHS[0], "// collapsed\n");

  const violations = checkCollapsedPaths(rootDir);

  assert.equal(violations.length, 1);
  assert.equal(violations[0].path, COLLAPSED_PATHS[0]);
  assert.equal(violations[0].kind, "collapsed_path");
});

test("ratcheted line caps apply only to decomposed parent files", () => {
  const rootDir = makeRoot();
  const capped = RATCHETED_FILE_LIMITS.find((entry) => entry.path.endsWith("sessions/handle.rs"));
  writeFile(rootDir, capped.path, lines(capped.limit + 1));
  writeFile(rootDir, "core/crates/ctx-daemon/src/daemon/unrelated.rs", lines(1000));

  const violations = checkRatchetedFileCaps(rootDir);

  assert.deepEqual(violations.map((entry) => entry.path), [capped.path]);
  assert.equal(violations[0].lineCount, capped.limit + 1);
});

test("cargo dependency parser finds simple dependencies, package aliases, and dependency tables", () => {
  const dependencies = parseCargoDependencies(`
    [package]
    name = "ctx-example-service"

    [dependencies]
    ctx-http = { path = "../ctx-http" }
    daemon_alias = { package = "ctx-daemon", path = "../ctx-daemon" }

    [dependencies.axum]
    workspace = true

    [target.'cfg(unix)'.dependencies]
    store_alias = { package = "ctx-store", path = "../ctx-store" }

    [dev-dependencies]
    ctx-http = { path = "../ctx-http" }
  `);

  assert.deepEqual(
    dependencies.map((entry) => entry.name),
    ["ctx-http", "daemon_alias", "ctx-daemon", "axum", "store_alias", "ctx-store"],
  );
});

test("cargo dependency parser finds package aliases in multiline inline tables", () => {
  const dependencies = parseCargoDependencies(`
    [package]
    name = "ctx-example-service"

    [dependencies]
    daemon_alias = {
      package = "ctx-daemon",
      path = "../ctx-daemon",
    }
  `);

  assert.deepEqual(
    dependencies.map((entry) => entry.name),
    ["daemon_alias", "ctx-daemon"],
  );
});

test("packageNameFromCargoToml reads package names", () => {
  assert.equal(packageNameFromCargoToml('[package]\nname = "ctx-session-service"\n'), "ctx-session-service");
});

test("crate classifiers cover service and runtime owners", () => {
  assert.equal(isServiceOrRuntimeCrate("ctx-session-service"), true);
  assert.equal(isServiceOrRuntimeCrate("ctx-workspace-services"), true);
  assert.equal(isServiceOrRuntimeCrate("ctx-transport-runtime"), true);
  assert.equal(isServiceOrRuntimeCrate("ctx-provider-runtime"), true);
  assert.equal(isServiceOrRuntimeCrate("ctx-core"), false);
});

test("package shape boundary classifier covers existing and planned service owners", () => {
  assert.equal(PACKAGE_SHAPE_BOUNDARY_CRATES.has("ctx-org-policy"), true);
  assert.equal(PACKAGE_SHAPE_BOUNDARY_CRATES.has("ctx-workspace-attachments"), true);
  assert.equal(PACKAGE_SHAPE_FORBIDDEN_BACKEDGE_DEPS.has("ctx-daemon"), true);
  assert.equal(isPackageShapeBoundaryCrate("ctx-workspace-stream-service"), true);
  assert.equal(isPackageShapeBoundaryCrate("ctx-task-service"), true);
  assert.equal(isPackageShapeBoundaryCrate("ctx-subagent-service"), true);
  assert.equal(isPackageShapeBoundaryCrate("ctx-run-scheduler"), true);
  assert.equal(isPackageShapeBoundaryCrate("ctx-session-runner"), true);
  assert.equal(isPackageShapeBoundaryCrate("ctx-core"), false);
});

test("workspace active snapshot forbids daemon, HTTP, store, and runtime dependencies", () => {
  assert.equal(isWorkspaceActiveSnapshotForbiddenDependency("ctx-daemon"), true);
  assert.equal(isWorkspaceActiveSnapshotForbiddenDependency("ctx-http-auth"), true);
  assert.equal(isWorkspaceActiveSnapshotForbiddenDependency("ctx-store"), true);
  assert.equal(isWorkspaceActiveSnapshotForbiddenDependency("ctx-transport-runtime"), true);
  assert.equal(isWorkspaceActiveSnapshotForbiddenDependency("ctx-provider-runtime"), true);
  assert.equal(isWorkspaceActiveSnapshotForbiddenDependency("ctx-core"), false);
});

test("cargo dependency direction rejects service, transport runtime, and active snapshot backedges", () => {
  const rootDir = makeRoot();
  writeFile(rootDir, "core/crates/ctx-example-service/Cargo.toml", `
    [package]
    name = "ctx-example-service"

    [dependencies]
    axum.workspace = true
    daemon_alias = { package = "ctx-daemon", path = "../ctx-daemon" }
    http_alias = {
      package = "ctx-http",
      path = "../ctx-http",
    }
    ctx-http = { path = "../ctx-http" }
  `);
  writeFile(rootDir, "core/crates/ctx-transport-runtime/Cargo.toml", `
    [package]
    name = "ctx-transport-runtime"

    [dependencies]
    ctx-store = { path = "../ctx-store" }
  `);
  writeFile(rootDir, "core/crates/ctx-workspace-active-snapshot/Cargo.toml", `
    [package]
    name = "ctx-workspace-active-snapshot"

    [dependencies]
    ctx-provider-runtime = { path = "../ctx-provider-runtime" }
    ctx-http-auth = { path = "../ctx-http-auth" }
  `);

  const messages = checkCargoDependencyDirection(rootDir).map((entry) => entry.message);

  assert.equal(messages.some((message) => message.includes("ctx-example-service must not depend on axum")), true);
  assert.equal(messages.some((message) => message.includes("ctx-example-service must not depend on ctx-daemon")), true);
  assert.equal(messages.some((message) => message.includes("ctx-example-service must not depend on ctx-http")), true);
  assert.equal(messages.some((message) => message.includes("ctx-transport-runtime must not depend on ctx-store")), true);
  assert.equal(messages.some((message) => message.includes("found ctx-provider-runtime")), true);
  assert.equal(messages.some((message) => message.includes("found ctx-http-auth")), true);
});

test("cargo dependency direction rejects explicit package-shape crate backedges", () => {
  const rootDir = makeRoot();
  writeFile(rootDir, "core/crates/ctx-org-policy/Cargo.toml", `
    [package]
    name = "ctx-org-policy"

    [dependencies]
    ctx-daemon = { path = "../ctx-daemon" }
  `);
  writeFile(rootDir, "core/crates/ctx-workspace-stream-service/Cargo.toml", `
    [package]
    name = "ctx-workspace-stream-service"

    [dependencies]
    axum.workspace = true
  `);
  writeFile(rootDir, "core/crates/ctx-run-scheduler/Cargo.toml", `
    [package]
    name = "ctx-run-scheduler"

    [dependencies]
    ctx-http = { path = "../ctx-http" }
  `);

  const messages = checkCargoDependencyDirection(rootDir).map((entry) => entry.message);

  assert.equal(messages.some((message) => message.includes("ctx-org-policy must not depend on ctx-daemon")), true);
  assert.equal(messages.some((message) => message.includes("ctx-workspace-stream-service must not depend on axum")), true);
  assert.equal(messages.some((message) => message.includes("ctx-run-scheduler must not depend on ctx-http")), true);
});

test("cargo direction does not reject dev-dependency-only test helpers", () => {
  const rootDir = makeRoot();
  writeFile(rootDir, "core/crates/ctx-example-service/Cargo.toml", `
    [package]
    name = "ctx-example-service"

    [dependencies]
    ctx-core = { path = "../ctx-core" }

    [dev-dependencies]
    ctx-http = { path = "../ctx-http" }
    axum.workspace = true
  `);

  assert.deepEqual(checkCargoDependencyDirection(rootDir), []);
});

test("head projection purity rejects daemon, HTTP, store, provider, transport, and runtime imports", () => {
  const rootDir = makeRoot();
  writeFile(rootDir, "core/crates/ctx-session-service/src/head_projection/mod.rs", `
    use ctx_daemon::daemon::DaemonHandle;
    use ctx_http::api::router;
    use ctx_store::Store;
    use ctx_provider_runtime::Runtime;
    use ctx_providers::ProviderCatalog;
    use ctx_transport_runtime::terminals;
    use ctx_workspace_runtime::WorkspaceRuntime;
  `);

  const kinds = checkHeadProjectionPurity(rootDir).map((entry) => entry.kind);

  assert.equal(kinds.length, 5);
  assert.equal(kinds.every((kind) => kind === "head_projection_import"), true);
});

test("head projection purity ignores line comments", () => {
  const rootDir = makeRoot();
  writeFile(rootDir, "core/crates/ctx-session-service/src/head_projection/mod.rs", `
    // use ctx_store::Store;
    use ctx_core::models::Session;
  `);

  assert.deepEqual(checkHeadProjectionPurity(rootDir), []);
});

test("full evaluator aggregates all static decomposition violations", () => {
  const rootDir = makeRoot();
  writeFile(rootDir, COLLAPSED_PATHS[1], "// collapsed\n");
  writeFile(rootDir, RATCHETED_FILE_LIMITS[0].path, lines(RATCHETED_FILE_LIMITS[0].limit + 10));
  writeFile(rootDir, "core/crates/ctx-example-runtime/Cargo.toml", `
    [package]
    name = "ctx-example-runtime"

    [dependencies]
    ctx-http = { path = "../ctx-http" }
  `);
  writeFile(rootDir, "core/crates/ctx-session-service/src/head_projection/mod.rs", "use ctx_store::Store;\n");

  const violations = evaluateDecompositionBoundaries(rootDir).violations;

  assert.deepEqual(
    violations.map((entry) => entry.kind).sort(),
    ["cargo_dependency", "collapsed_path", "file_cap", "head_projection_import"],
  );
});

test("run returns nonzero and prints architectural guidance when violations exist", () => {
  const rootDir = makeRoot();
  writeFile(rootDir, COLLAPSED_PATHS[2], "// collapsed\n");
  let output = "";
  const stream = {
    write(chunk) {
      output += String(chunk);
    },
  };

  const exitCode = run({
    rootDir,
    stdout: stream,
    stderr: stream,
  });

  assert.equal(exitCode, 1);
  assert.match(output, /ctx daemon decomposition boundary violations/u);
  assert.match(output, /Do not add compatibility shims/u);
});

test("run succeeds for an empty fixture tree", () => {
  const exitCode = run({
    rootDir: makeRoot(),
    stdout: { write() {} },
    stderr: { write() {} },
  });

  assert.equal(exitCode, 0);
});
