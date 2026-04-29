import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";

import {
  buildPlaywrightArgs,
  buildPlaywrightEnv,
  normalizeSpec,
  parseArgs,
  resolveExistingPath,
  resolveRepoRoot,
} from "./run-e2e-bazel-runtime.mjs";

describe("run-e2e-bazel-runtime", () => {
  it("normalizes suite spec paths", () => {
    expect(normalizeSpec("workbench-index.spec.ts")).toBe("e2e/workbench-index.spec.ts");
    expect(normalizeSpec("e2e/workbench-index.spec.ts")).toBe("e2e/workbench-index.spec.ts");
    expect(() => normalizeSpec("../outside.spec.ts")).toThrow(/invalid E2E spec path/);
  });

  it("parses strict Bazel runtime inputs", () => {
    expect(parseArgs([
      "--config",
      "playwright.premerge.config.ts",
      "--runtime-profile",
      "workbench-lite",
      "--ctx-http-bin",
      "ctx",
      "--spec",
      "e2e/workbench-index.spec.ts",
      "--",
      "--list",
    ])).toMatchObject({
      config: "playwright.premerge.config.ts",
      ctxHttpBin: "ctx",
      forwardedArgs: ["--list"],
      runtimeProfile: "workbench-lite",
      specs: ["e2e/workbench-index.spec.ts"],
    });
    expect(() => parseArgs([
      "--runtime-profile",
      "agent-full",
      "--ctx-http-bin",
      "ctx",
      "--spec",
      "e2e/workbench-index.spec.ts",
    ])).toThrow(/missing --config/);
    expect(() => parseArgs([
      "--config",
      "playwright.premerge.config.ts",
      "--runtime-profile",
      "agent-full",
      "--ctx-http-bin",
      "ctx",
      "--spec",
      "e2e/workbench-index.spec.ts",
    ])).toThrow(/requires --ctx-mcp-bin/);
  });

  it("does not expose the MCP-disabled flag as a test-author input", () => {
    const env = buildPlaywrightEnv({
      ctxHttpBin: "/tmp/ctx",
      env: {
        CTX_MCP_COMMAND: "/tmp/ambient-mcp",
        CTX_MCP_DISABLED: "0",
      },
      runtimeProfile: "workbench-lite",
      tempRoot: "/tmp",
      webDistDir: "/tmp/dist",
    });

    expect(env.CTX_E2E_RUNTIME_SOURCE).toBe("bazel-runfiles");
    expect(env.CTX_E2E_CTX_HTTP_BIN).toBe("/tmp/ctx");
    expect(env.CTX_E2E_CTX_MCP_BIN).toBeUndefined();
    expect(env.CTX_MCP_COMMAND).toBeUndefined();
    expect(env.CTX_MCP_DISABLED).toBeUndefined();
  });

  it("requires ctx-mcp only for the agent-full runtime profile", () => {
    const env = buildPlaywrightEnv({
      ctxHttpBin: "/tmp/ctx",
      ctxMcpBin: "/tmp/ctx-mcp",
      env: {},
      runtimeProfile: "agent-full",
      tempRoot: "/tmp",
      webDistDir: "/tmp/dist",
    });
    expect(env.CTX_E2E_CTX_MCP_BIN).toBe("/tmp/ctx-mcp");
  });

  it("roots the Bazel data dir inside the Bazel tmp dir for server cleanup safety", () => {
    const env = buildPlaywrightEnv({
      ctxHttpBin: "/tmp/ctx",
      env: {},
      runtimeProfile: "workbench-lite",
      tempRoot: "/tmp/ctx-e2e-bazel-root",
      webDistDir: "/tmp/dist",
    });
    const relative = path.relative(env.CTX_E2E_TMPDIR, env.CTX_E2E_DATA_DIR);

    expect(relative).not.toBe("");
    expect(relative.startsWith("..")).toBe(false);
    expect(path.isAbsolute(relative)).toBe(false);
    expect(path.basename(env.CTX_E2E_DATA_DIR)).toMatch(/^ctx-e2e-workbench-lite-data-/);
  });

  it("builds Playwright args without broad suite fallback", () => {
    expect(buildPlaywrightArgs({
      config: "playwright.premerge.config.ts",
      forwardedArgs: ["--grep", "unarchive"],
      specs: ["e2e/workbench-unarchive-visible.spec.ts"],
    })).toEqual([
      "test",
      "-c",
      "playwright.premerge.config.ts",
      "e2e/workbench-unarchive-visible.spec.ts",
      "--grep",
      "unarchive",
    ]);
  });

  it("resolves declared runfile inputs relative to TEST_SRCDIR", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-web-e2e-runfiles-"));
    const file = path.join(root, "ctx_monorepo", "core", "crates", "ctx-http", "ctx");
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, "#!/bin/sh\n");

    expect(resolveExistingPath("core/crates/ctx-http/ctx", {
      env: {
        TEST_SRCDIR: root,
        TEST_WORKSPACE: "ctx_monorepo",
      },
    })).toBe(file);
  });

  it("resolves the repo root from a core package candidate", () => {
    const repoRoot = path.resolve(__dirname, "../../../..");
    expect(resolveRepoRoot({}, path.join(repoRoot, "core"))).toBe(repoRoot);
  });
});
