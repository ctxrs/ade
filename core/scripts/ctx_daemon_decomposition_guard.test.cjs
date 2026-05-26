const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  COLLAPSED_PATHS,
  COLLAPSED_DIRECTORIES,
  CTX_HTTP_CLI_ONLY_SERVICE_DEPS,
  CTX_HTTP_FORBIDDEN_DOMAIN_SERVICE_DEPS,
  DAEMON_ROOT_ROUTE_FACADE_TARGETS,
  DAEMON_ROOT_WEB_SESSION_TRANSPORT_FACADE_DENY,
  DAEMON_HANDLE_STORE_LOOKUP_HOME,
  DAEMON_HANDLE_STORE_LOOKUP_SYMBOLS,
  MESSAGE_SERVICE_FORBIDDEN_DEPS,
  RATCHETED_FILE_LIMITS,
  PACKAGE_SHAPE_BOUNDARY_CRATES,
  PACKAGE_SHAPE_FORBIDDEN_BACKEDGE_DEPS,
  REPO_ONBOARDING_SERVICE_FORBIDDEN_DEPS,
  SESSION_RUNTIME_FORBIDDEN_DEPS,
  SESSION_VCS_SERVICE_FORBIDDEN_DEPS,
  TITLE_SERVICE_FORBIDDEN_DEPS,
  WORKSPACE_ATTACHMENTS_FORBIDDEN_DEPS,
  WORKSPACE_SERVICES_FORBIDDEN_DEPS,
  WORKTREE_BOOTSTRAP_SERVICE_FORBIDDEN_DEPS,
  WORKTREE_VCS_SERVICE_FORBIDDEN_DEPS,
  checkCargoDependencyDirection,
  checkCollapsedPaths,
  checkCtxHttpCliOnlyServiceUsage,
  checkDaemonRootRouteFacades,
  checkDaemonHandleStoreLookupOwnership,
  checkHeadProjectionPurity,
  checkRatchetedFileCaps,
  countLines,
  evaluateDecompositionBoundaries,
  isPackageShapeBoundaryCrate,
  isRouteContractsForbiddenDependency,
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

test("retired workspace-services attachment paths are rejected when recreated", () => {
  const rootDir = makeRoot();
  const retired = "core/crates/ctx-workspace-services/src/workspace_attachments.rs";
  assert.equal(COLLAPSED_PATHS.includes(retired), true);
  writeFile(rootDir, retired, "// retired attachment policy\n");

  const violations = checkCollapsedPaths(rootDir);

  assert(
    violations.some((violation) => violation.path === retired && violation.kind === "collapsed_path"),
  );
});

test("retired workspace-services attachment directory is rejected when recreated", () => {
  const rootDir = makeRoot();
  const retiredDir = "core/crates/ctx-workspace-services/src/workspace_attachments";
  assert.equal(COLLAPSED_DIRECTORIES.includes(retiredDir), true);
  writeFile(rootDir, `${retiredDir}/new_file.rs`, "// retired attachment policy\n");

  const violations = checkCollapsedPaths(rootDir);

  assert(
    violations.some((violation) => violation.path === retiredDir && violation.kind === "collapsed_path"),
  );
});

test("retired workspace-services repo onboarding directory is rejected when recreated", () => {
  const rootDir = makeRoot();
  const retiredDir = "core/crates/ctx-workspace-services/src/repo_onboarding";
  assert.equal(COLLAPSED_DIRECTORIES.includes(retiredDir), true);
  writeFile(rootDir, `${retiredDir}/new_file.rs`, "// retired repo onboarding policy\n");

  const violations = checkCollapsedPaths(rootDir);

  assert(
    violations.some((violation) => violation.path === retiredDir && violation.kind === "collapsed_path"),
  );
});

test("retired workspace-services crate directory is rejected when recreated", () => {
  const rootDir = makeRoot();
  const retiredDir = "core/crates/ctx-workspace-services";
  assert.equal(COLLAPSED_DIRECTORIES.includes(retiredDir), true);
  writeFile(rootDir, `${retiredDir}/Cargo.toml`, "[package]\nname = \"ctx-workspace-services\"\n");

  const violations = checkCollapsedPaths(rootDir);

  assert.equal(violations.length, 1);
  assert.equal(violations[0].path, retiredDir);
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

test("task route handle split module has a line-cap ratchet", () => {
  const rootDir = makeRoot();
  const capped = RATCHETED_FILE_LIMITS.find((entry) =>
    entry.path === "core/crates/ctx-daemon/src/daemon/task_route_handles.rs"
  );
  assert.ok(capped, "expected task_route_handles.rs to have a line cap");
  writeFile(rootDir, capped.path, lines(capped.limit + 1));

  const violations = checkRatchetedFileCaps(rootDir);

  assert.deepEqual(violations.map((entry) => entry.path), [capped.path]);
  assert.equal(violations[0].lineCount, capped.limit + 1);
});

test("session route handle split module has a line-cap ratchet", () => {
  const rootDir = makeRoot();
  const capped = RATCHETED_FILE_LIMITS.find((entry) =>
    entry.path === "core/crates/ctx-daemon/src/daemon/session_route_handles.rs"
  );
  assert.ok(capped, "expected session_route_handles.rs to have a line cap");
  writeFile(rootDir, capped.path, lines(capped.limit + 1));

  const violations = checkRatchetedFileCaps(rootDir);

  assert.deepEqual(violations.map((entry) => entry.path), [capped.path]);
  assert.equal(violations[0].lineCount, capped.limit + 1);
});

test("workspace stream route handle split module has a line-cap ratchet", () => {
  const rootDir = makeRoot();
  const capped = RATCHETED_FILE_LIMITS.find((entry) =>
    entry.path === "core/crates/ctx-daemon/src/daemon/workspace_stream_route_handles.rs"
  );
  assert.ok(capped, "expected workspace_stream_route_handles.rs to have a line cap");
  writeFile(rootDir, capped.path, lines(capped.limit + 1));

  const violations = checkRatchetedFileCaps(rootDir);

  assert.deepEqual(violations.map((entry) => entry.path), [capped.path]);
  assert.equal(violations[0].lineCount, capped.limit + 1);
});

test("launch route handle split module has a line-cap ratchet", () => {
  const rootDir = makeRoot();
  const capped = RATCHETED_FILE_LIMITS.find((entry) =>
    entry.path === "core/crates/ctx-daemon/src/daemon/launch_route_handles.rs"
  );
  assert.ok(capped, "expected launch_route_handles.rs to have a line cap");
  writeFile(rootDir, capped.path, lines(capped.limit + 1));

  const violations = checkRatchetedFileCaps(rootDir);

  assert.deepEqual(violations.map((entry) => entry.path), [capped.path]);
  assert.equal(violations[0].lineCount, capped.limit + 1);
});

test("maintenance route handle split module has a line-cap ratchet", () => {
  const rootDir = makeRoot();
  const capped = RATCHETED_FILE_LIMITS.find((entry) =>
    entry.path === "core/crates/ctx-daemon/src/daemon/maintenance_route_handles.rs"
  );
  assert.ok(capped, "expected maintenance_route_handles.rs to have a line cap");
  writeFile(rootDir, capped.path, lines(capped.limit + 1));

  const violations = checkRatchetedFileCaps(rootDir);

  assert.deepEqual(violations.map((entry) => entry.path), [capped.path]);
  assert.equal(violations[0].lineCount, capped.limit + 1);
});

test("mobile route handle split module has a line-cap ratchet", () => {
  const rootDir = makeRoot();
  const capped = RATCHETED_FILE_LIMITS.find((entry) =>
    entry.path === "core/crates/ctx-daemon/src/daemon/mobile_route_handles.rs"
  );
  assert.ok(capped, "expected mobile_route_handles.rs to have a line cap");
  writeFile(rootDir, capped.path, lines(capped.limit + 1));

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
  assert.equal(isPackageShapeBoundaryCrate("ctx-mobile-access-service"), true);
  assert.equal(isPackageShapeBoundaryCrate("ctx-route-contracts"), true);
  assert.equal(isPackageShapeBoundaryCrate("ctx-run-archive-service"), true);
  assert.equal(isPackageShapeBoundaryCrate("ctx-run-scheduler"), true);
  assert.equal(isPackageShapeBoundaryCrate("ctx-session-artifacts"), true);
  assert.equal(isPackageShapeBoundaryCrate("ctx-session-message-service"), true);
  assert.equal(isPackageShapeBoundaryCrate("ctx-session-runtime"), true);
  assert.equal(isPackageShapeBoundaryCrate("ctx-session-runner"), true);
  assert.equal(isPackageShapeBoundaryCrate("ctx-session-title-service"), true);
  assert.equal(isPackageShapeBoundaryCrate("ctx-worktree-bootstrap-service"), true);
  assert.equal(isPackageShapeBoundaryCrate("ctx-worktree-vcs-service"), true);
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

test("route contracts forbid daemon, HTTP, Axum, and runtime dependencies", () => {
  assert.equal(isRouteContractsForbiddenDependency("ctx-daemon"), true);
  assert.equal(isRouteContractsForbiddenDependency("ctx-http"), true);
  assert.equal(isRouteContractsForbiddenDependency("axum"), true);
  assert.equal(isRouteContractsForbiddenDependency("ctx-transport-runtime"), true);
  assert.equal(isRouteContractsForbiddenDependency("ctx-core"), false);
  assert.equal(isRouteContractsForbiddenDependency("serde"), false);
});

test("message service boundary rejects reverse coupling to session orchestration", () => {
  assert.equal(MESSAGE_SERVICE_FORBIDDEN_DEPS.has("ctx-session-service"), true);
  assert.equal(MESSAGE_SERVICE_FORBIDDEN_DEPS.has("ctx-daemon"), true);

  const rootDir = makeRoot();
  writeFile(rootDir, "core/crates/ctx-session-message-service/Cargo.toml", `
    [package]
    name = "ctx-session-message-service"

    [dependencies]
    ctx-session-service = { path = "../ctx-session-service" }
  `);

  const messages = checkCargoDependencyDirection(rootDir).map((entry) => entry.message);

  assert.equal(
    messages.some((message) => message.includes("ctx-session-message-service must not depend on ctx-session-service")),
    true,
  );
});

test("session runtime boundary rejects reverse coupling to orchestration and runtime adapters", () => {
  assert.equal(SESSION_RUNTIME_FORBIDDEN_DEPS.has("ctx-session-service"), true);
  assert.equal(SESSION_RUNTIME_FORBIDDEN_DEPS.has("ctx-workspace-active-snapshot"), true);

  const rootDir = makeRoot();
  writeFile(rootDir, "core/crates/ctx-session-runtime/Cargo.toml", `
    [package]
    name = "ctx-session-runtime"

    [dependencies]
    ctx-session-service = { path = "../ctx-session-service" }
    ctx-workspace-active-snapshot = { path = "../ctx-workspace-active-snapshot" }
  `);

  const messages = checkCargoDependencyDirection(rootDir).map((entry) => entry.message);

  assert.equal(
    messages.some((message) => message.includes("ctx-session-runtime must not depend on ctx-session-service")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-session-runtime must not depend on ctx-workspace-active-snapshot")),
    true,
  );
});

test("session title service boundary rejects route and orchestration coupling", () => {
  assert.equal(TITLE_SERVICE_FORBIDDEN_DEPS.has("ctx-session-service"), true);
  assert.equal(TITLE_SERVICE_FORBIDDEN_DEPS.has("ctx-route-contracts"), true);

  const rootDir = makeRoot();
  writeFile(rootDir, "core/crates/ctx-session-title-service/Cargo.toml", `
    [package]
    name = "ctx-session-title-service"

    [dependencies]
    ctx-session-service = { path = "../ctx-session-service" }
    ctx-route-contracts = { path = "../ctx-route-contracts" }
  `);

  const messages = checkCargoDependencyDirection(rootDir).map((entry) => entry.message);

  assert.equal(
    messages.some((message) => message.includes("ctx-session-title-service must not depend on ctx-session-service")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-session-title-service must not depend on ctx-route-contracts")),
    true,
  );
});

test("session VCS service boundary rejects orchestration and raw workspace IO coupling", () => {
  assert.equal(SESSION_VCS_SERVICE_FORBIDDEN_DEPS.has("ctx-session-service"), true);
  assert.equal(SESSION_VCS_SERVICE_FORBIDDEN_DEPS.has("ctx-workspace-services"), true);
  assert.equal(SESSION_VCS_SERVICE_FORBIDDEN_DEPS.has("ctx-worktree-vcs-service"), true);

  const rootDir = makeRoot();
  writeFile(rootDir, "core/crates/ctx-session-vcs-service/Cargo.toml", `
    [package]
    name = "ctx-session-vcs-service"

    [dependencies]
    ctx-core = { path = "../ctx-core" }

    [dev-dependencies]
    ctx-session-service = { path = "../ctx-session-service" }

    [target.'cfg(test)'.dev-dependencies]
    ctx-workspace-services = { path = "../ctx-workspace-services" }
    ctx-worktree-vcs-service = { path = "../ctx-worktree-vcs-service" }
  `);

  const messages = checkCargoDependencyDirection(rootDir).map((entry) => entry.message);

  assert.equal(
    messages.some((message) => message.includes("ctx-session-vcs-service must not depend on ctx-session-service")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-session-vcs-service must not depend on ctx-workspace-services")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-session-vcs-service must not depend on ctx-worktree-vcs-service")),
    true,
  );
});

test("worktree VCS service boundary rejects session, workspace, daemon, route, HTTP, and runtime coupling", () => {
  assert.equal(WORKTREE_VCS_SERVICE_FORBIDDEN_DEPS.has("ctx-session-vcs-service"), true);
  assert.equal(WORKTREE_VCS_SERVICE_FORBIDDEN_DEPS.has("ctx-workspace-services"), true);
  assert.equal(WORKTREE_VCS_SERVICE_FORBIDDEN_DEPS.has("ctx-worktree-data-plane"), true);
  assert.equal(WORKTREE_VCS_SERVICE_FORBIDDEN_DEPS.has("ctx-workspace-container"), true);
  assert.equal(WORKTREE_VCS_SERVICE_FORBIDDEN_DEPS.has("ctx-harness-runtime"), true);

  const rootDir = makeRoot();
  writeFile(rootDir, "core/crates/ctx-worktree-vcs-service/Cargo.toml", `
    [package]
    name = "ctx-worktree-vcs-service"

    [dependencies]
    ctx-core = { path = "../ctx-core" }
    ctx-session-vcs-service = { path = "../ctx-session-vcs-service" }
    ctx-workspace-services = { path = "../ctx-workspace-services" }
    ctx-worktree-data-plane = { path = "../ctx-worktree-data-plane" }
    ctx-workspace-container = { path = "../ctx-workspace-container" }
    ctx-harness-runtime = { path = "../ctx-harness-runtime" }

    [dev-dependencies]
    ctx-http = { path = "../ctx-http" }

    [target.'cfg(test)'.dev-dependencies]
    ctx-route-contracts = { path = "../ctx-route-contracts" }
  `);

  const messages = checkCargoDependencyDirection(rootDir).map((entry) => entry.message);

  assert.equal(
    messages.some((message) => message.includes("ctx-worktree-vcs-service must not depend on ctx-session-vcs-service")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-worktree-vcs-service must not depend on ctx-workspace-services")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-worktree-vcs-service must not depend on ctx-worktree-data-plane")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-worktree-vcs-service must not depend on ctx-workspace-container")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-worktree-vcs-service must not depend on ctx-harness-runtime")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-worktree-vcs-service must not depend on ctx-http")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-worktree-vcs-service must not depend on ctx-route-contracts")),
    true,
  );
});

test("worktree bootstrap service boundary rejects daemon runtime, route, config, store, and container coupling", () => {
  assert.equal(WORKTREE_BOOTSTRAP_SERVICE_FORBIDDEN_DEPS.has("ctx-workspace-config"), true);
  assert.equal(WORKTREE_BOOTSTRAP_SERVICE_FORBIDDEN_DEPS.has("ctx-worktree-data-plane"), true);
  assert.equal(WORKTREE_BOOTSTRAP_SERVICE_FORBIDDEN_DEPS.has("ctx-execution-runtime"), true);
  assert.equal(WORKTREE_BOOTSTRAP_SERVICE_FORBIDDEN_DEPS.has("ctx-harness-runtime"), true);
  assert.equal(WORKTREE_BOOTSTRAP_SERVICE_FORBIDDEN_DEPS.has("ctx-workspace-runtime"), true);

  const rootDir = makeRoot();
  writeFile(rootDir, "core/crates/ctx-worktree-bootstrap-service/Cargo.toml", `
    [package]
    name = "ctx-worktree-bootstrap-service"

    [dependencies]
    ctx-core = { path = "../ctx-core" }
    ctx-workspace-config = { path = "../ctx-workspace-config" }
    ctx-store = { path = "../ctx-store" }
    ctx-worktree-data-plane = { path = "../ctx-worktree-data-plane" }
    ctx-workspace-runtime = { path = "../ctx-workspace-runtime" }

    [dev-dependencies]
    ctx-http = { path = "../ctx-http" }

    [target.'cfg(test)'.dev-dependencies]
    ctx-harness-runtime = { path = "../ctx-harness-runtime" }
    ctx-execution-runtime = { path = "../ctx-execution-runtime" }
  `);

  const messages = checkCargoDependencyDirection(rootDir).map((entry) => entry.message);

  assert.equal(
    messages.some((message) => message.includes("ctx-worktree-bootstrap-service must not depend on ctx-workspace-config")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-worktree-bootstrap-service must not depend on ctx-store")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-worktree-bootstrap-service must not depend on ctx-worktree-data-plane")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-worktree-bootstrap-service must not depend on ctx-http")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-worktree-bootstrap-service must not depend on ctx-harness-runtime")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-worktree-bootstrap-service must not depend on ctx-workspace-runtime")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-worktree-bootstrap-service must not depend on ctx-execution-runtime")),
    true,
  );
});

test("workspace attachments boundary rejects broad workspace-services backedge", () => {
  assert.equal(WORKSPACE_ATTACHMENTS_FORBIDDEN_DEPS.has("ctx-workspace-services"), true);

  const rootDir = makeRoot();
  writeFile(rootDir, "core/crates/ctx-workspace-attachments/Cargo.toml", `
    [package]
    name = "ctx-workspace-attachments"

    [dependencies]
    ctx-core = { path = "../ctx-core" }
    ctx-workspace-services = { path = "../ctx-workspace-services" }
  `);

  const messages = checkCargoDependencyDirection(rootDir).map((entry) => entry.message);

  assert.equal(
    messages.some((message) => message.includes("ctx-workspace-attachments must not depend on ctx-workspace-services")),
    true,
  );
});

test("repo onboarding service boundary rejects route, broad workspace, runtime, and HTTP bypass coupling", () => {
  assert.equal(REPO_ONBOARDING_SERVICE_FORBIDDEN_DEPS.has("ctx-workspace-services"), true);
  assert.equal(REPO_ONBOARDING_SERVICE_FORBIDDEN_DEPS.has("ctx-route-contracts"), true);
  assert.equal(REPO_ONBOARDING_SERVICE_FORBIDDEN_DEPS.has("ctx-workspace-runtime"), true);
  assert.equal(WORKSPACE_SERVICES_FORBIDDEN_DEPS.has("ctx-repo-onboarding-service"), true);
  assert.equal(CTX_HTTP_CLI_ONLY_SERVICE_DEPS.has("ctx-repo-onboarding-service"), true);
  assert.equal(CTX_HTTP_FORBIDDEN_DOMAIN_SERVICE_DEPS.has("ctx-repo-onboarding-service"), false);
  assert.equal(CTX_HTTP_FORBIDDEN_DOMAIN_SERVICE_DEPS.has("ctx-worktree-vcs-service"), true);

  const rootDir = makeRoot();
  writeFile(rootDir, "core/crates/ctx-repo-onboarding-service/Cargo.toml", `
    [package]
    name = "ctx-repo-onboarding-service"

    [dependencies]
    ctx-core = { path = "../ctx-core" }
    ctx-workspace-services = { path = "../ctx-workspace-services" }
    ctx-route-contracts = { path = "../ctx-route-contracts" }

    [dev-dependencies]
    ctx-workspace-runtime = { path = "../ctx-workspace-runtime" }
  `);
  writeFile(rootDir, "core/crates/ctx-workspace-services/Cargo.toml", `
    [package]
    name = "ctx-workspace-services"

    [dependencies]
    ctx-repo-onboarding-service = { path = "../ctx-repo-onboarding-service" }
  `);
  writeFile(rootDir, "core/crates/ctx-http/Cargo.toml", `
    [package]
    name = "ctx-http"

    [dev-dependencies]
    ctx-repo-onboarding-service = { path = "../ctx-repo-onboarding-service" }
    ctx-worktree-vcs-service = { path = "../ctx-worktree-vcs-service" }
  `);

  const messages = checkCargoDependencyDirection(rootDir).map((entry) => entry.message);

  assert.equal(
    messages.some((message) => message.includes("ctx-repo-onboarding-service must not depend on ctx-workspace-services")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-repo-onboarding-service must not depend on ctx-route-contracts")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-repo-onboarding-service must not depend on ctx-workspace-runtime")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-workspace-services must not depend on ctx-repo-onboarding-service")),
    true,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-http must not depend on ctx-repo-onboarding-service")),
    false,
  );
  assert.equal(
    messages.some((message) => message.includes("ctx-http must not depend on ctx-worktree-vcs-service")),
    true,
  );
});

test("ctx-http allows repo onboarding service only from CLI main", () => {
  const rootDir = makeRoot();
  writeFile(rootDir, "core/crates/ctx-http/src/main.rs", `
    fn main() {
      let _ = ctx_repo_onboarding_service::init_workspace;
    }
  `);
  writeFile(rootDir, "core/crates/ctx-http/src/api/repo.rs", `
    use ctx_repo_onboarding_service::init_workspace;
  `);
  writeFile(rootDir, "core/crates/ctx-http/src/lib.rs", `
    fn bad() {
      let _ = ctx_repo_onboarding_service::init_workspace;
    }
  `);

  const violations = checkCtxHttpCliOnlyServiceUsage(rootDir);

  assert.deepEqual(
    violations.map((entry) => entry.path),
    ["core/crates/ctx-http/src/api/repo.rs", "core/crates/ctx-http/src/lib.rs"],
  );
  assert(
    violations.every((entry) =>
      entry.message.includes("ctx-repo-onboarding-service is allowed in ctx-http only for the CLI entrypoint"),
    ),
  );
});

test("daemon root route facade guard rejects route-contract reexports", () => {
  const rootDir = makeRoot();
  writeFile(rootDir, "core/crates/ctx-daemon/src/daemon/tasks.rs", `
    pub use route_contract::{TaskRouteResponse as PublicTaskRouteResponse};
  `);
  writeFile(rootDir, "core/crates/ctx-daemon/src/daemon/merge_queue.rs", `
    pub use self::route_contract::*;
  `);
  writeFile(rootDir, "core/crates/ctx-daemon/src/daemon/terminals.rs", `
    pub use route_contract
      ::TerminalRouteError;
  `);
  writeFile(rootDir, "core/crates/ctx-daemon/src/daemon/web_sessions.rs", `
    pub use crate::daemon::web_sessions::route_contract::*;
  `);

  const violations = checkDaemonRootRouteFacades(rootDir);

  assert.equal(
    DAEMON_ROOT_ROUTE_FACADE_TARGETS.includes("core/crates/ctx-daemon/src/daemon/tasks.rs"),
    true,
  );
  assert.deepEqual(
    violations.map((entry) => entry.path).sort(),
    [
      "core/crates/ctx-daemon/src/daemon/merge_queue.rs",
      "core/crates/ctx-daemon/src/daemon/tasks.rs",
      "core/crates/ctx-daemon/src/daemon/terminals.rs",
      "core/crates/ctx-daemon/src/daemon/web_sessions.rs",
    ],
  );
  assert(
    violations.every((entry) =>
      entry.message.includes("must not publicly reexport route_contract symbols"),
    ),
  );
});

test("daemon root route facade guard rejects web-session transport leaf reexports", () => {
  const rootDir = makeRoot();
  writeFile(rootDir, "core/crates/ctx-daemon/src/daemon/web_sessions.rs", `
    pub use ctx_transport_runtime::web_sessions::{
      WebSessionActionError,
      WebSessionSignalUpstream as PublicSignalUpstream,
    };
    pub use ctx_transport_runtime::web_sessions::*;
    use ctx_transport_runtime::web_sessions::WebSessionViewPage;
  `);

  const violations = checkDaemonRootRouteFacades(rootDir);

  assert.equal(DAEMON_ROOT_WEB_SESSION_TRANSPORT_FACADE_DENY.has("WebSessionActionError"), true);
  assert.deepEqual(
    violations.map((entry) => entry.message),
    [
      "core/crates/ctx-daemon/src/daemon/web_sessions.rs must not publicly reexport WebSessionActionError from ctx_transport_runtime::web_sessions; use the owner crate directly.",
      "core/crates/ctx-daemon/src/daemon/web_sessions.rs must not publicly reexport WebSessionSignalUpstream from ctx_transport_runtime::web_sessions; use the owner crate directly.",
      "core/crates/ctx-daemon/src/daemon/web_sessions.rs must not publicly glob-reexport ctx_transport_runtime::web_sessions; use owner-crate symbols directly.",
    ],
  );
});

test("daemon handle store lookup ownership rejects definitions returning to handle.rs", () => {
  const rootDir = makeRoot();
  writeFile(rootDir, DAEMON_HANDLE_STORE_LOOKUP_HOME, `
    pub(in crate::daemon) struct ProtectedWorkspaceStoreLookup {}
    impl ProtectedWorkspaceStoreLookup {
      fn store_for_workspace() {}
    }
    pub(in crate::daemon) struct SessionStoreLookup {}
    impl SessionStoreLookup {}
    pub(in crate::daemon) struct TaskStoreLookup {}
    impl TaskStoreLookup {}
    pub(in crate::daemon) fn session_store_access_anyhow() {}
    async fn reject_archived_subagent_session() {}
    fn is_transient_store_open_error() {}
    fn scoped_mcp_session_store_error() {}
  `);

  const violations = checkDaemonHandleStoreLookupOwnership(rootDir);

  assert.deepEqual(
    violations.map((violation) => violation.kind),
    Array.from(
      { length: DAEMON_HANDLE_STORE_LOOKUP_SYMBOLS.length + 3 },
      () => "daemon_handle_store_lookup_ownership",
    ),
  );
});

test("daemon handle store lookup ownership rejects old handle imports", () => {
  const rootDir = makeRoot();
  writeFile(rootDir, DAEMON_HANDLE_STORE_LOOKUP_HOME, "pub struct OtherHandle;\n");
  writeFile(rootDir, "core/crates/ctx-daemon/src/daemon/sessions/demo_seed.rs", `
    use crate::daemon::handle::{ProtectedWorkspaceStoreLookup, SessionStoreLookup};
  `);
  writeFile(rootDir, "core/crates/ctx-daemon/src/daemon/workspaces/attachments.rs", `
    use crate::daemon::handle::TaskStoreLookup;
    fn helper() {
      let _ = crate::daemon::handle::session_store_access_anyhow;
    }
  `);

  const violations = checkDaemonHandleStoreLookupOwnership(rootDir);

  assert.deepEqual(
    violations.map((violation) => violation.message.match(/^([A-Za-z_][A-Za-z0-9_]*)/u)?.[1]),
    [
      "ProtectedWorkspaceStoreLookup",
      "SessionStoreLookup",
      "TaskStoreLookup",
      "session_store_access_anyhow",
    ],
  );
});

test("daemon handle store lookup ownership allows state-owned imports and handle references", () => {
  const rootDir = makeRoot();
  writeFile(rootDir, DAEMON_HANDLE_STORE_LOOKUP_HOME, `
    use super::state::{ProtectedWorkspaceStoreLookup, SessionStoreLookup, TaskStoreLookup};

    fn assemble() {
      let _ = ProtectedWorkspaceStoreLookup::new;
      let _ = SessionStoreLookup::new;
      let _ = TaskStoreLookup::new;
    }
  `);
  writeFile(rootDir, "core/crates/ctx-daemon/src/daemon/sessions/demo_seed.rs", `
    use crate::daemon::{ProtectedWorkspaceStoreLookup, SessionStoreLookup};
  `);
  writeFile(rootDir, "core/crates/ctx-daemon/src/daemon/state/store_lookup.rs", `
    pub(in crate::daemon) struct ProtectedWorkspaceStoreLookup {}
    pub(in crate::daemon) struct SessionStoreLookup {}
    pub(in crate::daemon) struct TaskStoreLookup {}
    pub(in crate::daemon) fn session_store_access_anyhow() {}
    async fn reject_archived_subagent_session() {}
    fn is_transient_store_open_error() {}
    fn scoped_mcp_session_store_error() {}
  `);

  assert.deepEqual(checkDaemonHandleStoreLookupOwnership(rootDir), []);
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
  writeFile(rootDir, "core/crates/ctx-route-contracts/Cargo.toml", `
    [package]
    name = "ctx-route-contracts"

    [dependencies]
    ctx-transport-runtime = { path = "../ctx-transport-runtime" }
  `);

  const messages = checkCargoDependencyDirection(rootDir).map((entry) => entry.message);

  assert.equal(messages.some((message) => message.includes("ctx-org-policy must not depend on ctx-daemon")), true);
  assert.equal(messages.some((message) => message.includes("ctx-workspace-stream-service must not depend on axum")), true);
  assert.equal(messages.some((message) => message.includes("ctx-run-scheduler must not depend on ctx-http")), true);
  assert.equal(messages.some((message) => message.includes("ctx-route-contracts must stay DTO-only")), true);
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
  writeFile(rootDir, "core/crates/ctx-session-runtime/src/head_projection/mod.rs", `
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
  writeFile(rootDir, "core/crates/ctx-session-runtime/src/head_projection/mod.rs", `
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
  writeFile(rootDir, "core/crates/ctx-session-runtime/src/head_projection/mod.rs", "use ctx_store::Store;\n");

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
