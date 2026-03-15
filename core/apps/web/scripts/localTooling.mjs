import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const pnpmCommand = process.platform === "win32" ? "pnpm.cmd" : "pnpm";
const lockedInstallArgs = [
  "install",
  "--frozen-lockfile",
  "--config.enable-modules-dir=true",
  "--modules-dir=node_modules",
];

const binNameForTool = (tool) => (process.platform === "win32" ? `${tool}.cmd` : tool);

const isWorkspaceRoot = (dir) => fs.existsSync(path.join(dir, "pnpm-workspace.yaml"));

export const resolveWorkspaceRoot = (startDir) => {
  let current = path.resolve(startDir);
  while (true) {
    if (isWorkspaceRoot(current)) return current;
    const parent = path.dirname(current);
    if (parent === current) return path.resolve(startDir);
    current = parent;
  }
};

export const lockedInstallHint = (installRoot) =>
  `bash -lc "cd ${installRoot} && pnpm ${lockedInstallArgs.join(" ")}"`;

export const resolveLocalNodeBin = (packageRoot, tool) => {
  const expectedBin = binNameForTool(tool);
  let current = path.resolve(packageRoot);
  while (true) {
    const candidate = path.join(current, "node_modules", ".bin", expectedBin);
    if (fs.existsSync(candidate)) return candidate;
    const parent = path.dirname(current);
    if (parent === current) break;
    current = parent;
  }
  return path.join(path.resolve(packageRoot), "node_modules", ".bin", expectedBin);
};

export const ensureLockedNodeInstall = (packageRoot) => {
  const installRoot = resolveWorkspaceRoot(packageRoot);
  const result = spawnSync(pnpmCommand, lockedInstallArgs, {
    cwd: installRoot,
    env: process.env,
    stdio: "inherit",
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
};

export const requireLocalNodeBin = (packageRoot, tool) => {
  const binPath = resolveLocalNodeBin(packageRoot, tool);
  if (!fs.existsSync(binPath)) {
    const installRoot = resolveWorkspaceRoot(packageRoot);
    throw new Error(`Missing local ${tool} binary for ${packageRoot}; run '${lockedInstallHint(installRoot)}'`);
  }
  return binPath;
};
