const assert = require("node:assert/strict");
const test = require("node:test");

const {
  API_DOMAIN_RAW_STORE_PATTERNS,
  API_RAW_DAEMON_PATTERNS,
  HANDLE_BACKDOOR_PATTERNS,
  apiPatternsForPath,
  isTestRustPath,
  scanRepo,
  scanText,
  stripCfgTestItems,
} = require("./ctx_http_daemon_boundary_guard.cjs");

test("daemon boundary guard rejects raw daemon state in API code", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/example.rs",
    contents: `
      use crate::daemon::DaemonState;
      async fn handler(State(state): State<Arc<DaemonState>>) {}
    `,
    patterns: API_RAW_DAEMON_PATTERNS,
  });

  assert.equal(violations.length, 4);
  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "raw DaemonState type",
      "raw DaemonState type",
      "raw daemon state extractor",
      "raw daemon state arc",
    ],
  );
});

test("daemon boundary guard rejects broad daemon handle access in API code", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/example.rs",
    contents: `
      async fn handler(State(state): State<DaemonHandle>) {
        let _ = state.core().daemon_handle();
      }
    `,
    patterns: API_RAW_DAEMON_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "broad daemon handle extractor",
      "daemon handle escalation call",
    ],
  );
});

test("daemon boundary guard rejects route-visible store accessors", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/example.rs",
    contents: `
      async fn handler(State(core): State<CoreHandle>, State(sessions): State<SessionsHandle>) {
        let _ = core.global_store().get_workspace(workspace_id).await;
        let _ = sessions.store_for_session(session_id).await;
        let _ = sessions.route_session_store(session_id).await;
        let _ = workspaces.route_existing_workspace_store(workspace_id).await;
        let _ = sessions.load_session_store(session_id).await;
        let _ = sessions.existing_session_store_for_write(session_id).await;
      }
    `,
    patterns: API_RAW_DAEMON_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "global store accessor",
      "daemon store accessor",
      "daemon store accessor",
      "daemon store accessor",
      "daemon store accessor",
      "daemon store accessor",
    ],
  );
});

test("daemon boundary guard rejects raw Store in daemon-blind API families", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/sessions/example.rs",
    contents: `
      use ctx_store::Store;
      async fn handler(store: &Store) {}
    `,
    patterns: API_DOMAIN_RAW_STORE_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    [
      "raw ctx_store Store in daemon-blind API family",
      "raw ctx_store Store in daemon-blind API family",
    ],
  );
});

test("daemon boundary guard scopes raw Store ban to migrated API families", () => {
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/sessions/example.rs").includes(
      API_DOMAIN_RAW_STORE_PATTERNS[0],
    ),
    true,
  );
  assert.equal(
    apiPatternsForPath("core/crates/ctx-http/src/api/tasks/example.rs").includes(
      API_DOMAIN_RAW_STORE_PATTERNS[0],
    ),
    false,
  );
});

test("daemon boundary guard rejects broad daemon handle fields", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/example.rs",
    contents: `
      struct ProxyState {
        handle: DaemonHandle,
      }
    `,
    patterns: API_RAW_DAEMON_PATTERNS,
  });

  assert.deepEqual(
    violations.map((violation) => violation.name),
    ["broad daemon handle field"],
  );
});

test("daemon boundary guard allows cfg(test) helpers to construct daemon state", () => {
  const stripped = stripCfgTestItems(`
    #[cfg(test)]
    fn helper(state: &Arc<DaemonState>) {
      let _ = state;
    }

    fn production_handler(State(state): State<CoreHandle>) {}
  `);

  const violations = scanText({
    filePath: "core/crates/ctx-http/src/api/example.rs",
    contents: stripped,
    patterns: API_RAW_DAEMON_PATTERNS,
  });

  assert.deepEqual(violations, []);
});

test("daemon boundary guard ignores test paths", () => {
  assert.equal(isTestRustPath("core/crates/ctx-http/src/api/tasks/lifecycle_tests/archive.rs"), true);
  assert.equal(isTestRustPath("core/crates/ctx-http/src/api/workspaces/tests.rs"), true);
  assert.equal(isTestRustPath("core/crates/ctx-http/src/api/workspaces/management.rs"), false);
});

test("daemon boundary guard rejects DaemonHandle raw-state backdoors", () => {
  const violations = scanText({
    filePath: "core/crates/ctx-http/src/daemon/handle.rs",
    contents: `
      impl FromRef<DaemonHandle> for Arc<DaemonState> {}
      impl DaemonHandle {
        fn state(&self) -> &Arc<DaemonState> { todo!() }
      }
      impl CoreHandle {
        fn daemon_handle(&self) -> DaemonHandle { todo!() }
      }
      fn proxy(handle: &DaemonHandle) {
        let router: Arc<axum::Router> = Arc::new(router(handle.clone()));
      }
    `,
    patterns: HANDLE_BACKDOOR_PATTERNS,
  });

  assert.equal(violations.length, 4);
});

test("checked-in ctx-http API satisfies the daemon boundary", () => {
  assert.deepEqual(scanRepo(), []);
});
