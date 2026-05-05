import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import {
  buildPlaywrightArgs,
  buildPlaywrightEnv,
  envWithCurrentNodeOnPath,
  materializeBazelWebRepo,
  normalizeSpec,
  parseArgs,
  prepareRuntimeRepoRoot,
  pathsReferToSameFile,
  resolveExistingPath,
  resolveLocalNodeBin,
  resolveRepoRoot,
} from "./run-e2e-bazel-runtime.mjs";

describe("run-e2e-bazel-runtime", () => {
  it("normalizes suite spec paths", () => {
    assert.equal(normalizeSpec("workbench-index.spec.ts"), "e2e/workbench-index.spec.ts");
    assert.equal(normalizeSpec("e2e/workbench-index.spec.ts"), "e2e/workbench-index.spec.ts");
    assert.throws(() => normalizeSpec("../outside.spec.ts"), /invalid E2E spec path/u);
  });

  it("parses strict Bazel runtime inputs", () => {
    assert.deepEqual(parseArgs([
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
    ]), {
      config: "playwright.premerge.config.ts",
      ctxHttpBin: "ctx",
      ctxMcpBin: "",
      forwardedArgs: ["--list"],
      runtimeProfile: "workbench-lite",
      specs: ["e2e/workbench-index.spec.ts"],
      suite: "",
    });
    assert.throws(() => parseArgs([
      "--runtime-profile",
      "agent-full",
      "--ctx-http-bin",
      "ctx",
      "--spec",
      "e2e/workbench-index.spec.ts",
    ]), /missing --config/u);
    assert.throws(() => parseArgs([
      "--config",
      "playwright.premerge.config.ts",
      "--runtime-profile",
      "agent-full",
      "--ctx-http-bin",
      "ctx",
      "--spec",
      "e2e/workbench-index.spec.ts",
    ]), /requires --ctx-mcp-bin/u);
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

    assert.equal(env.CTX_E2E_RUNTIME_SOURCE, "bazel-runfiles");
    assert.equal(env.CTX_E2E_CTX_HTTP_BIN, "/tmp/ctx");
    assert.equal(env.CTX_E2E_CTX_MCP_BIN, undefined);
    assert.equal(env.CTX_MCP_COMMAND, undefined);
    assert.equal(env.CTX_MCP_DISABLED, undefined);
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
    assert.equal(env.CTX_E2E_CTX_MCP_BIN, "/tmp/ctx-mcp");
  });

  it("keeps package shims runnable under Bazel's sanitized PATH", () => {
    const env = envWithCurrentNodeOnPath({ PATH: "/usr/bin" });
    assert.equal(env.PATH.split(path.delimiter)[0], path.dirname(process.execPath));
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

    assert.notEqual(relative, "");
    assert.equal(relative.startsWith(".."), false);
    assert.equal(path.isAbsolute(relative), false);
    assert.match(path.basename(env.CTX_E2E_DATA_DIR), /^ctx-e2e-workbench-lite-data-/u);
  });

  it("builds Playwright args without broad suite fallback", () => {
    assert.deepEqual(buildPlaywrightArgs({
      config: "playwright.premerge.config.ts",
      forwardedArgs: ["--grep", "unarchive"],
      specs: ["e2e/workbench-unarchive-visible.spec.ts"],
    }), [
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

    assert.equal(resolveExistingPath("core/crates/ctx-http/ctx", {
      env: {
        TEST_SRCDIR: root,
        TEST_WORKSPACE: "ctx_monorepo",
      },
    }), file);
  });

  it("resolves the repo root from a core package candidate", () => {
    const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-web-e2e-repo-"));
    fs.mkdirSync(path.join(repoRoot, "core", "apps", "web"), { recursive: true });
    fs.writeFileSync(path.join(repoRoot, "core", "package.json"), "{}\n");
    fs.writeFileSync(path.join(repoRoot, "core", "apps", "web", "package.json"), "{}\n");

    assert.equal(resolveRepoRoot({}, path.join(repoRoot, "core")), repoRoot);
  });

  it("prefers runfiles over the checkout when Bazel runfile metadata is present", () => {
    const checkoutRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-web-e2e-checkout-"));
    const runfilesRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-web-e2e-runfiles-root-"));
    for (const root of [checkoutRoot, runfilesRoot]) {
      fs.mkdirSync(path.join(root, "core", "apps", "web"), { recursive: true });
      fs.writeFileSync(path.join(root, "core", "package.json"), "{}\n");
      fs.writeFileSync(path.join(root, "core", "apps", "web", "package.json"), "{}\n");
    }

    assert.equal(resolveRepoRoot({
      BUILD_WORKSPACE_DIRECTORY: checkoutRoot,
      RUNFILES_DIR: path.dirname(runfilesRoot),
      TEST_WORKSPACE: path.basename(runfilesRoot),
    }, checkoutRoot), runfilesRoot);
  });

  it("materializes runfile-backed web sources before invoking Vite", () => {
    const sourceRepoRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-web-e2e-source.runfiles-"));
    const targetFile = path.join(sourceRepoRoot, "checkout-source.ts");
    const sourceCoreRoot = path.join(sourceRepoRoot, "core");
    const sourceWebRoot = path.join(sourceCoreRoot, "apps", "web");
    const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-web-e2e-materialized-"));
    fs.mkdirSync(path.join(sourceWebRoot, "src"), { recursive: true });
    fs.mkdirSync(path.join(sourceWebRoot, "node_modules", ".bin"), { recursive: true });
    fs.mkdirSync(path.join(sourceWebRoot, "test-results"), { recursive: true });
    fs.mkdirSync(path.join(sourceCoreRoot, "node_modules", ".aspect_rules_js"), { recursive: true });
    fs.mkdirSync(path.join(sourceCoreRoot, "scripts", "lib"), { recursive: true });
    fs.writeFileSync(path.join(sourceCoreRoot, "package.json"), "{}\n");
    fs.writeFileSync(path.join(sourceCoreRoot, "scripts", "lib", "cache_roots.cjs"), "module.exports = {};\n");
    fs.writeFileSync(targetFile, "export const source = 'materialized';\n");
    fs.symlinkSync(targetFile, path.join(sourceWebRoot, "src", "App.ts"));
    fs.writeFileSync(path.join(sourceWebRoot, "test-results", "stale.txt"), "stale\n");

    const materializedRepoRoot = materializeBazelWebRepo({
      repoRoot: sourceRepoRoot,
      runtimeProfile: "agent-full",
      tempRoot,
    });
    const materializedCoreRoot = path.join(materializedRepoRoot, "core");
    const materializedAppSource = path.join(materializedCoreRoot, "apps", "web", "src", "App.ts");

    assert.equal(fs.readFileSync(materializedAppSource, "utf8"), "export const source = 'materialized';\n");
    assert.equal(fs.lstatSync(materializedAppSource).isSymbolicLink(), false);
    assert.equal(fs.lstatSync(path.join(materializedCoreRoot, "node_modules")).isSymbolicLink(), true);
    assert.equal(fs.lstatSync(path.join(materializedCoreRoot, "apps", "web", "node_modules")).isSymbolicLink(), true);
    assert.equal(fs.existsSync(path.join(materializedCoreRoot, "apps", "web", "test-results", "stale.txt")), false);
    assert.equal(fs.existsSync(path.join(materializedCoreRoot, "scripts", "lib", "cache_roots.cjs")), true);
  });

  it("only materializes the runtime repo when Bazel runfiles are in use", () => {
    const checkoutRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-web-e2e-checkout-root-"));
    const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-web-e2e-runtime-root-"));
    fs.mkdirSync(path.join(checkoutRoot, "core", "apps", "web"), { recursive: true });
    fs.writeFileSync(path.join(checkoutRoot, "core", "package.json"), "{}\n");
    fs.writeFileSync(path.join(checkoutRoot, "core", "apps", "web", "package.json"), "{}\n");

    assert.equal(prepareRuntimeRepoRoot({
      env: {},
      repoRoot: checkoutRoot,
      runtimeProfile: "workbench-lite",
      tempRoot,
    }), checkoutRoot);

    const runfilesRoot = `${checkoutRoot}.runfiles/_main`;
    fs.mkdirSync(path.join(runfilesRoot, "core", "apps", "web"), { recursive: true });
    fs.writeFileSync(path.join(runfilesRoot, "core", "package.json"), "{}\n");
    fs.writeFileSync(path.join(runfilesRoot, "core", "apps", "web", "package.json"), "{}\n");
    const prepared = prepareRuntimeRepoRoot({
      env: {},
      repoRoot: runfilesRoot,
      runtimeProfile: "workbench-lite",
      tempRoot,
    });

    assert.notEqual(prepared, runfilesRoot);
    assert.equal(fs.existsSync(path.join(prepared, "core", "apps", "web", "package.json")), true);
  });

  it("resolves package bin entries when runfiles do not contain .bin shims", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-web-e2e-bin-"));
    const webRoot = path.join(root, "core", "apps", "web");
    const packageRoot = path.join(webRoot, "node_modules", "playwright");
    const cli = path.join(packageRoot, "cli.js");
    fs.mkdirSync(packageRoot, { recursive: true });
    fs.writeFileSync(
      path.join(packageRoot, "package.json"),
      JSON.stringify({ bin: { playwright: "cli.js" } }),
    );
    fs.writeFileSync(cli, "#!/usr/bin/env node\n");

    assert.equal(resolveLocalNodeBin(webRoot, "playwright"), cli);
  });

  it("treats runfiles symlinks as direct module invocations", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-web-e2e-entrypoint-"));
    const target = path.join(root, "run-e2e-bazel-runtime.mjs");
    const link = path.join(root, "runfiles-link.mjs");
    fs.writeFileSync(target, "export {};\n");
    fs.symlinkSync(target, link);

    assert.equal(pathsReferToSameFile(link, target), true);
    assert.equal(pathsReferToSameFile(path.join(root, "missing.mjs"), target), false);
  });
});
