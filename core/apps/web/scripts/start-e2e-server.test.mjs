import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";

import {
  ensureCargoTargetDir,
  resolveCargoTargetDir,
  resolveE2EWebDistDir,
  resolveServeWebDistDir,
  resolveWebBuildArgs,
  shouldUseConfiguredCtxMcpCommand,
} from "./start-e2e-server.mjs";

describe("start-e2e-server", () => {
  it("scopes web dist under the temp root", () => {
    const resolved = resolveE2EWebDistDir("/tmp/ctx-tests", 4242);
    expect(resolved).toBe(path.resolve("/tmp/ctx-tests", "ctx-e2e-web-dist-4242"));
  });

  it("targets an isolated dist dir and empties it", () => {
    const resolved = resolveWebBuildArgs("/tmp/ctx-web-dist");
    expect(resolved).toEqual([
      "build",
      "--outDir",
      "/tmp/ctx-web-dist",
      "--emptyOutDir",
    ]);
  });

  it("honors an explicit CTX_WEB_DIST override", () => {
    const resolved = resolveServeWebDistDir("/repo/core", {
      CTX_WEB_DIST: "custom/web-dist",
    });
    expect(resolved).toBe(path.resolve("/repo/core", "custom/web-dist"));
  });

  it("falls back to the checked-in web dist when skip-build is enabled", () => {
    const resolved = resolveServeWebDistDir("/repo/core", {}, true);
    expect(resolved).toBe(path.resolve("/repo/core", "apps", "web", "dist"));
  });

  it("requires an explicit e2e opt-in before using a configured ctx-mcp command", () => {
    expect(
      shouldUseConfiguredCtxMcpCommand({
        CTX_MCP_COMMAND: "/Applications/ctx.app/Contents/Resources/bin/ctx-mcp",
      }),
    ).toBe(false);
    expect(
      shouldUseConfiguredCtxMcpCommand({
        CTX_MCP_COMMAND: "/tmp/custom-ctx-mcp",
        CTX_E2E_ALLOW_CONFIGURED_MCP_COMMAND: "1",
      }),
    ).toBe(true);
  });

  it("resolves a relative cargo target dir against the repo root", () => {
    expect(resolveCargoTargetDir("/repo/core", { CARGO_TARGET_DIR: "../tmp/cargo" })).toBe(
      path.resolve("/repo/core", "../tmp/cargo"),
    );
  });

  it("creates the cargo target dir before invoking cargo", () => {
    const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-start-e2e-cargo-"));
    const targetDir = ensureCargoTargetDir(repoRoot, { CARGO_TARGET_DIR: "tmp/cargo" });
    expect(targetDir).toBe(path.resolve(repoRoot, "tmp/cargo"));
    expect(fs.statSync(targetDir).isDirectory()).toBe(true);
  });
});
