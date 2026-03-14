import test from "node:test";
import assert from "node:assert/strict";
import path from "node:path";

import {
  resolveE2EWebDistDir,
  resolveServeWebDistDir,
  resolveWebBuildArgs,
  shouldUseConfiguredCtxMcpCommand,
} from "./start-e2e-server.mjs";

test("resolveE2EWebDistDir scopes web dist under the temp root", () => {
  const resolved = resolveE2EWebDistDir("/tmp/ctx-tests", 4242);
  assert.equal(resolved, path.resolve("/tmp/ctx-tests", "ctx-e2e-web-dist-4242"));
});

test("resolveWebBuildArgs targets an isolated dist dir and empties it", () => {
  const resolved = resolveWebBuildArgs("/tmp/ctx-web-dist");
  assert.deepEqual(resolved, [
    "-C",
    "apps/web",
    "exec",
    "vite",
    "build",
    "--outDir",
    "/tmp/ctx-web-dist",
    "--emptyOutDir",
  ]);
});

test("resolveServeWebDistDir honors an explicit CTX_WEB_DIST override", () => {
  const resolved = resolveServeWebDistDir("/repo/core", {
    CTX_WEB_DIST: "custom/web-dist",
  });
  assert.equal(resolved, path.resolve("/repo/core", "custom/web-dist"));
});

test("resolveServeWebDistDir falls back to the checked-in web dist when skip-build is enabled", () => {
  const resolved = resolveServeWebDistDir("/repo/core", {}, true);
  assert.equal(resolved, path.resolve("/repo/core", "apps", "web", "dist"));
});

test("shouldUseConfiguredCtxMcpCommand requires an explicit e2e opt-in", () => {
  assert.equal(
    shouldUseConfiguredCtxMcpCommand({
      CTX_MCP_COMMAND: "/Applications/ctx.app/Contents/Resources/bin/ctx-mcp",
    }),
    false,
  );
  assert.equal(
    shouldUseConfiguredCtxMcpCommand({
      CTX_MCP_COMMAND: "/tmp/custom-ctx-mcp",
      CTX_E2E_ALLOW_CONFIGURED_MCP_COMMAND: "1",
    }),
    true,
  );
});
