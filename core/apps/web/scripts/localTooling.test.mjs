import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { expect, test } from "vitest";

import {
  ensureLockedNodeInstall,
  ensurePlaywrightBrowserInstall,
  lockedInstallHint,
  requireLocalNodeBin,
  resolveLocalNodeBin,
  resolveWorkspaceRoot,
} from "./localTooling.mjs";
import { resolveWebBuildArgs } from "./start-e2e-server.mjs";

test("resolveLocalNodeBin points at the local .bin entry", () => {
  const packageRoot = "/tmp/ctx-web";
  const expectedBin = process.platform === "win32" ? "playwright.cmd" : "playwright";
  expect(resolveLocalNodeBin(packageRoot, "playwright")).toBe(
    path.join(packageRoot, "node_modules", ".bin", expectedBin),
  );
});

test("resolveWorkspaceRoot walks up to the pnpm workspace root", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-local-tooling-workspace-"));
  const packageRoot = path.join(root, "apps", "web");
  fs.mkdirSync(packageRoot, { recursive: true });
  fs.writeFileSync(path.join(root, "pnpm-workspace.yaml"), "packages:\n  - apps/*\n");
  expect(resolveWorkspaceRoot(packageRoot)).toBe(root);
});

test("resolveLocalNodeBin falls back to workspace-root bins", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-local-tooling-bin-"));
  const packageRoot = path.join(root, "apps", "web");
  const binPath = path.join(root, "node_modules", ".bin", process.platform === "win32" ? "vite.cmd" : "vite");
  fs.mkdirSync(packageRoot, { recursive: true });
  fs.mkdirSync(path.dirname(binPath), { recursive: true });
  fs.writeFileSync(path.join(root, "pnpm-workspace.yaml"), "packages:\n  - apps/*\n");
  fs.writeFileSync(binPath, "#!/bin/sh\n");

  expect(resolveLocalNodeBin(packageRoot, "vite")).toBe(binPath);
  expect(requireLocalNodeBin(packageRoot, "vite")).toBe(binPath);
});

test("requireLocalNodeBin returns the local binary when present", () => {
  const packageRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-local-tooling-"));
  const binPath = resolveLocalNodeBin(packageRoot, "vite");
  fs.mkdirSync(path.dirname(binPath), { recursive: true });
  fs.writeFileSync(binPath, "#!/bin/sh\n");

  expect(requireLocalNodeBin(packageRoot, "vite")).toBe(binPath);
});

test("requireLocalNodeBin throws a clear install hint when missing", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-local-tooling-missing-"));
  const packageRoot = path.join(root, "apps", "web");
  fs.mkdirSync(packageRoot, { recursive: true });
  fs.writeFileSync(path.join(root, "pnpm-workspace.yaml"), "packages:\n  - apps/*\n");
  expect(() => requireLocalNodeBin(packageRoot, "playwright")).toThrow(
    new RegExp(
      `cd ${root.replace(/[.*+?^${}()|[\\]\\\\]/g, "\\$&")} && pnpm install --frozen-lockfile`,
    ),
  );
});

test("lockedInstallHint uses the workspace-root frozen-lockfile install", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-local-tooling-hint-"));
  const packageRoot = path.join(root, "apps", "web");
  fs.mkdirSync(packageRoot, { recursive: true });
  fs.writeFileSync(path.join(root, "pnpm-workspace.yaml"), "packages:\n  - apps/*\n");

  expect(lockedInstallHint(packageRoot)).toMatch(
    new RegExp(`cd ${root.replace(/[.*+?^${}()|[\\]\\\\]/g, "\\$&")} && pnpm install --frozen-lockfile`),
  );
});

test("ensureLockedNodeInstall installs from the workspace root without prompts", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-local-tooling-install-root-"));
  const packageRoot = path.join(root, "apps", "web");
  fs.mkdirSync(packageRoot, { recursive: true });
  fs.writeFileSync(path.join(root, "pnpm-workspace.yaml"), "packages:\n  - apps/*\n");

  const calls = [];
  ensureLockedNodeInstall(packageRoot, {
    env: { PATH: "/usr/bin" },
    spawnSyncImpl: (command, args, options) => {
      calls.push({ command, args, options });
      return { status: 0 };
    },
    exitImpl: (code) => {
      throw new Error(`unexpected exit ${code}`);
    },
  });

  expect(calls).toEqual([
    {
      command: process.platform === "win32" ? "pnpm.cmd" : "pnpm",
      args: ["install", "--frozen-lockfile"],
      options: {
        cwd: root,
        env: {
          PATH: "/usr/bin",
          CI: "1",
          npm_config_confirm_modules_purge: "false",
        },
        stdio: "inherit",
      },
    },
  ]);
});

test("ensurePlaywrightBrowserInstall skips install when Chromium is already present", () => {
  const packageRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-local-tooling-playwright-present-"));
  const playwrightBin = resolveLocalNodeBin(packageRoot, "playwright");
  fs.mkdirSync(path.dirname(playwrightBin), { recursive: true });
  fs.writeFileSync(playwrightBin, "#!/bin/sh\n");

  const calls = [];
  const executablePath = path.join(packageRoot, "ms-playwright", "chromium", "chrome");
  const spawnSyncImpl = (command, args) => {
    calls.push([command, args]);
    return { status: 0, stdout: executablePath };
  };

  expect(
    ensurePlaywrightBrowserInstall(packageRoot, {
      spawnSyncImpl,
      existsSyncImpl: (candidate) => candidate === executablePath,
      nodeExecPath: "/fake/node",
    }),
  ).toBe(executablePath);
  expect(calls).toEqual([
    ["/fake/node", ["-e", "const { chromium } = require('playwright'); process.stdout.write(chromium.executablePath())"]],
  ]);
});

test("ensurePlaywrightBrowserInstall installs Chromium when missing", () => {
  const packageRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-local-tooling-playwright-install-"));
  const playwrightBin = resolveLocalNodeBin(packageRoot, "playwright");
  fs.mkdirSync(path.dirname(playwrightBin), { recursive: true });
  fs.writeFileSync(playwrightBin, "#!/bin/sh\n");

  const executablePath = path.join(packageRoot, "ms-playwright", "chromium", "chrome");
  const calls = [];
  let installed = false;
  const spawnSyncImpl = (command, args) => {
    calls.push([command, args]);
    if (command === "/fake/node") {
      return { status: 0, stdout: executablePath };
    }
    installed = true;
    return { status: 0 };
  };

  expect(
    ensurePlaywrightBrowserInstall(packageRoot, {
      spawnSyncImpl,
      existsSyncImpl: (candidate) => candidate === executablePath && installed,
      nodeExecPath: "/fake/node",
    }),
  ).toBe(executablePath);
  expect(calls).toEqual([
    ["/fake/node", ["-e", "const { chromium } = require('playwright'); process.stdout.write(chromium.executablePath())"]],
    [playwrightBin, ["install", "chromium"]],
    ["/fake/node", ["-e", "const { chromium } = require('playwright'); process.stdout.write(chromium.executablePath())"]],
  ]);
});

test("ensurePlaywrightBrowserInstall throws when install fails", () => {
  const packageRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-local-tooling-playwright-fail-"));
  const playwrightBin = resolveLocalNodeBin(packageRoot, "playwright");
  fs.mkdirSync(path.dirname(playwrightBin), { recursive: true });
  fs.writeFileSync(playwrightBin, "#!/bin/sh\n");

  const executablePath = path.join(packageRoot, "ms-playwright", "chromium", "chrome");
  const spawnSyncImpl = (command) => {
    if (command === "/fake/node") {
      return { status: 0, stdout: executablePath };
    }
    return { status: 1 };
  };

  expect(() =>
    ensurePlaywrightBrowserInstall(packageRoot, {
      spawnSyncImpl,
      existsSyncImpl: () => false,
      nodeExecPath: "/fake/node",
    }),
  ).toThrow(/Playwright browser install failed/);
});

test("resolveWebBuildArgs calls vite directly without pnpm exec indirection", () => {
  expect(resolveWebBuildArgs("/tmp/ctx-dist")).toEqual([
    "build",
    "--outDir",
    "/tmp/ctx-dist",
    "--emptyOutDir",
  ]);
});
