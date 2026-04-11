import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { expect, test } from "vitest";

import {
  browsersBySuite,
  buildSuiteRunnerEnv,
  normalizeForwardedArgs,
  runSuite,
} from "./run-e2e-suite.mjs";

test("normalizeForwardedArgs strips a leading passthrough separator", () => {
  expect(normalizeForwardedArgs(["--", "--list"])).toEqual(["--list"]);
  expect(normalizeForwardedArgs(["--list"])).toEqual(["--list"]);
});

test("buildSuiteRunnerEnv threads the shared Playwright cache path", () => {
  const volatileRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-run-e2e-suite-cache-"));
  const env = buildSuiteRunnerEnv({
    CTX_VOLATILE_ROOT: volatileRoot,
  });

  expect(env.CTX_VOLATILE_ROOT).toBe(volatileRoot);
  expect(env.PLAYWRIGHT_BROWSERS_PATH).toBe(path.join(volatileRoot, "cache", "playwright"));
  expect(fs.statSync(env.PLAYWRIGHT_BROWSERS_PATH).isDirectory()).toBe(true);
});

test("runSuite reuses the cache-scoped env for browser install and Playwright launch", () => {
  const volatileRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-run-e2e-suite-flow-"));
  const installCalls = [];
  const spawnCalls = [];
  const stderr = [];

  const exitCode = runSuite("premerge_required", ["--", "--list"], {
    env: { CTX_VOLATILE_ROOT: volatileRoot },
    ensureLockedNodeInstallImpl: () => {},
    ensurePlaywrightBrowserInstallImpl: (_root, options) => {
      installCalls.push(options);
      return {};
    },
    requireLocalNodeBinImpl: () => "/fake/playwright",
    spawnSyncImpl: (command, args, options) => {
      spawnCalls.push({ command, args, options });
      return { status: 0 };
    },
    stderr: (...parts) => {
      stderr.push(parts.join(" "));
    },
  });

  const expectedBrowserPath = path.join(volatileRoot, "cache", "playwright");
  expect(exitCode).toBe(0);
  expect(installCalls).toHaveLength(1);
  expect(installCalls[0]).toMatchObject({
    browsers: browsersBySuite.premerge_required,
    env: expect.objectContaining({
      CTX_VOLATILE_ROOT: volatileRoot,
      PLAYWRIGHT_BROWSERS_PATH: expectedBrowserPath,
    }),
  });
  expect(spawnCalls).toHaveLength(1);
  expect(spawnCalls[0]).toMatchObject({
    command: "/fake/playwright",
    options: expect.objectContaining({
      env: expect.objectContaining({
        CTX_VOLATILE_ROOT: volatileRoot,
        PLAYWRIGHT_BROWSERS_PATH: expectedBrowserPath,
      }),
    }),
  });
  expect(spawnCalls[0].args).toContain("--list");
  expect(stderr.some((line) => line.includes("running suite 'premerge_required'"))).toBe(true);
});
