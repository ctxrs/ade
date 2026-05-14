"use strict";

const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");

const {
  auditDesktopBrowserRouteContract,
  browserSecretAllowsRoute,
  classifyBrowserSecretPolicySource,
  expressionToRawPath,
  findApiCallsInSource,
  loadBrowserSecretPolicy,
  nonDesktopPrincipalRequirement,
  normalizeRoutePath,
  routeKey,
} = require("./desktop_browser_route_contract.cjs");

const repoRoot = path.resolve(__dirname, "..", "..");

test("desktop browser secret is owner-wide for transition and leaves no desktop web route blocked", () => {
  const result = auditDesktopBrowserRouteContract({ repoRoot });
  assert.equal(result.policy.mode, "desktop_web_owner");
  assert.equal(result.invalidPolicy, false);
  assert.deepEqual(
    result.blocked.map((route) => routeKey(route.method, route.path)),
    [],
  );
});

test("known former 401 desktop routes are covered by the owner-wide policy", () => {
  const policy = loadBrowserSecretPolicy(repoRoot);
  assert.equal(browserSecretAllowsRoute("POST", "/api/providers/codex/install", policy), true);
  assert.equal(browserSecretAllowsRoute("POST", "/api/repo/clone", policy), true);
  assert.equal(browserSecretAllowsRoute("POST", "/api/execution/launch/start", policy), true);
  assert.equal(browserSecretAllowsRoute("POST", "/api/mobile/access/enable", policy), true);
  assert.equal(browserSecretAllowsRoute("POST", "/api/sessions/abc/diff/apply", policy), true);
});

test("mobile-token-only handler routes are not classified as desktop browser routes", () => {
  const policy = loadBrowserSecretPolicy(repoRoot);
  assert.equal(browserSecretAllowsRoute("POST", "/api/mobile/register", policy), false);
  assert.equal(browserSecretAllowsRoute("POST", "/api/mobile/secure", policy), false);
  assert.equal(browserSecretAllowsRoute("GET", "/api/mobile/secure/workspaces/abc/stream", policy), false);
  assert.equal(
    nonDesktopPrincipalRequirement("POST", "/api/mobile/register")?.principal,
    "mobile_device_registration",
  );
});

test("browser secret policy detector fails closed if owner-wide route auth disappears", () => {
  const result = auditDesktopBrowserRouteContract({
    repoRoot,
    policy: { mode: "unknown", classification: null },
  });
  assert.equal(result.invalidPolicy, true);
  assert.ok(result.blocked.length > 0);
});

test("browser secret policy source detector requires exact owner-wide body", () => {
  assert.equal(
    classifyBrowserSecretPolicySource(`
      pub fn browser_query_secret_bearer_is_valid(
          path: &str,
          bearer_token: Option<&str>,
          auth_token: &str,
      ) -> bool {
          path.starts_with("/api/")
              && bearer_token.is_some_and(|value| value == derive_browser_query_secret(auth_token))
      }
    `).mode,
    "desktop_web_owner",
  );
  assert.equal(
    classifyBrowserSecretPolicySource(`
      pub fn browser_query_secret_bearer_is_valid(
          path: &str,
          bearer_token: Option<&str>,
          auth_token: &str,
      ) -> bool {
          path.starts_with("/api/")
              && path.ends_with("/readonly")
              && bearer_token.is_some_and(|value| value == derive_browser_query_secret(auth_token))
      }
    `).mode,
    "unknown",
  );
});

test("route extraction normalizes templates, query suffixes, and install target helper", () => {
  assert.equal(
    normalizeRoutePath(expressionToRawPath("withInstallTargetParam(`/api/providers/${providerId}/install`, target)")),
    "/api/providers/*/install",
  );
  assert.equal(
    normalizeRoutePath(expressionToRawPath("`/api/sessions/${sessionId}/snapshot${suffix}`")),
    "/api/sessions/*/snapshot",
  );
  assert.equal(
    normalizeRoutePath(expressionToRawPath("`/api/resource_utilization?workspace_id=${encodeURIComponent(workspaceId)}`")),
    "/api/resource_utilization",
  );
});

test("api call scanner handles typed apiAny calls and daemon URL builders", () => {
  const calls = findApiCallsInSource(
    `
      apiAny<Result>(withInstallTargetParam(\`/api/providers/\${providerId}/install\`, target), { method: "POST" });
      apiAny<Status>(\`/api/workspaces/\${workspaceId}/merge_queue_config\`, { method: "POST" });
      daemonFetchRaw(\`/api/worktrees/\${worktreeId}/bootstrap/logs\`);
      getDaemonWsUrl("/api/execution/launch/stream", qs);
      getDaemonHttpUrl(\`/api/blobs/\${encodeURIComponent(blobId)}\`);
    `,
    "fixture.ts",
  );
  assert.deepEqual(
    calls.map((call) => routeKey(call.method, call.path)),
    [
      "POST /api/providers/*/install",
      "POST /api/workspaces/*/merge_queue_config",
      "GET /api/worktrees/*/bootstrap/logs",
      "GET /api/blobs/*",
      "GET /api/execution/launch/stream",
    ],
  );
});
