import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { expect, test } from "vitest";

import {
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
      `cd ${packageRoot.replace(/[.*+?^${}()|[\\]\\\\]/g, "\\$&")} && pnpm install --frozen-lockfile`,
    ),
  );
});

test("lockedInstallHint uses the normal frozen-lockfile workspace install", () => {
  expect(lockedInstallHint("/tmp/ctx-root/apps/web")).toMatch(
    /cd \/tmp\/ctx-root\/apps\/web && pnpm install --frozen-lockfile/,
  );
});

test("resolveWebBuildArgs calls vite directly without pnpm exec indirection", () => {
  expect(resolveWebBuildArgs("/tmp/ctx-dist")).toEqual([
    "build",
    "--outDir",
    "/tmp/ctx-dist",
    "--emptyOutDir",
  ]);
});
