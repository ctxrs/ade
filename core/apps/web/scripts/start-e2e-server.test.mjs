import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";

import {
  ensureCargoTargetDir,
  ensureE2ETempDir,
  prepareE2EServerDirs,
  resolveCargoTargetDir,
  resolveE2EWebDistDir,
  resolveServeWebDistDir,
  resolveWebBuildArgs,
  shouldUseConfiguredCtxMcpCommand,
  shouldUseConfiguredWebDist,
} from "./start-e2e-server.mjs";

describe("start-e2e-server", () => {
  it("scopes web dist under the volatile artifact root", () => {
    const resolved = resolveE2EWebDistDir("/repo/core", {
      CTX_VOLATILE_ROOT: "/tmp/ctx-tests",
    });
    expect(resolved).toContain(path.resolve("/tmp/ctx-tests", "artifacts", "web-dist"));
    expect(resolved).toMatch(/dist$/);
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

  it("honors an explicit CTX_E2E_WEB_DIST override", () => {
    const resolved = resolveServeWebDistDir("/repo/core", {
      CTX_E2E_WEB_DIST: "custom/web-dist",
    });
    expect(resolved).toBe(path.resolve("/repo/core", "custom/web-dist"));
  });

  it("ignores an ambient CTX_WEB_DIST override without an e2e opt-in", () => {
    const resolved = resolveServeWebDistDir("/repo/core", {
      CTX_WEB_DIST: "/Applications/ctx.app/Contents/Resources/web/dist",
      CTX_VOLATILE_ROOT: "/tmp/ctx-tests",
    });
    expect(resolved).toBe(resolveE2EWebDistDir("/repo/core", {
      CTX_WEB_DIST: "/Applications/ctx.app/Contents/Resources/web/dist",
      CTX_VOLATILE_ROOT: "/tmp/ctx-tests",
    }));
  });

  it("allows CTX_WEB_DIST only when explicitly opted in for e2e", () => {
    const resolved = resolveServeWebDistDir("/repo/core", {
      CTX_WEB_DIST: "custom/web-dist",
      CTX_E2E_ALLOW_CONFIGURED_WEB_DIST: "1",
    });
    expect(resolved).toBe(path.resolve("/repo/core", "custom/web-dist"));
  });

  it("falls back to the checked-in web dist when skip-build is enabled", () => {
    const resolved = resolveServeWebDistDir("/repo/core", {}, true);
    expect(resolved).toBe(path.resolve("/repo/core", "apps", "web", "dist"));
  });

  it("requires an explicit e2e opt-in before using a configured web dist", () => {
    expect(
      shouldUseConfiguredWebDist({
        CTX_WEB_DIST: "/Applications/ctx.app/Contents/Resources/web/dist",
      }),
    ).toBe(false);
    expect(
      shouldUseConfiguredWebDist({
        CTX_WEB_DIST: "/tmp/custom-web-dist",
        CTX_E2E_ALLOW_CONFIGURED_WEB_DIST: "1",
      }),
    ).toBe(true);
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

  it("creates the e2e temp dir before build tooling uses TMPDIR", () => {
    const tempDir = ensureE2ETempDir(path.join(os.tmpdir(), "ctx-e2e-start-server-tmp"));
    expect(tempDir).toBe(path.resolve(os.tmpdir(), "ctx-e2e-start-server-tmp"));
    expect(fs.statSync(tempDir).isDirectory()).toBe(true);
  });

  it("cleans the data dir before startup without breaking an explicitly shared tmp dir", () => {
    const sharedDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-e2e-start-server-shared-"));
    const staleFile = path.join(sharedDir, "stale.txt");
    fs.writeFileSync(staleFile, "stale");

    const prepared = prepareE2EServerDirs(sharedDir, sharedDir);

    expect(prepared.dataDir).toBe(sharedDir);
    expect(prepared.tmpDir).toBe(sharedDir);
    expect(fs.statSync(sharedDir).isDirectory()).toBe(true);
    expect(fs.existsSync(staleFile)).toBe(false);

    const distDir = resolveE2EWebDistDir("/repo/core", {
      CTX_VOLATILE_ROOT: sharedDir,
    });
    fs.mkdirSync(distDir, { recursive: true });
    expect(fs.statSync(distDir).isDirectory()).toBe(true);
  });
});
