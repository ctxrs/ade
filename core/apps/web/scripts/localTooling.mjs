import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const pnpmCommand = process.platform === "win32" ? "pnpm.cmd" : "pnpm";
const lockedInstallArgs = ["install", "--frozen-lockfile"];

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

const resolveLockedInstallRoot = (packageRoot) => resolveWorkspaceRoot(packageRoot);

const resolveLockedInstallEnv = (env) => ({
  ...env,
  CI: env.CI ?? "1",
  npm_config_confirm_modules_purge: env.npm_config_confirm_modules_purge ?? "false",
});

export const lockedInstallHint = (packageRoot) =>
  `bash -lc "cd ${resolveLockedInstallRoot(packageRoot)} && pnpm ${lockedInstallArgs.join(" ")}"`;

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

export const ensureLockedNodeInstall = (
  packageRoot,
  {
    spawnSyncImpl = spawnSync,
    env = process.env,
    exitImpl = process.exit,
  } = {},
) => {
  const result = spawnSyncImpl(pnpmCommand, lockedInstallArgs, {
    cwd: resolveLockedInstallRoot(packageRoot),
    env: resolveLockedInstallEnv(env),
    stdio: "inherit",
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    exitImpl(result.status ?? 1);
  }
};

export const requireLocalNodeBin = (packageRoot, tool) => {
  const binPath = resolveLocalNodeBin(packageRoot, tool);
  if (!fs.existsSync(binPath)) {
    throw new Error(`Missing local ${tool} binary for ${packageRoot}; run '${lockedInstallHint(packageRoot)}'`);
  }
  return binPath;
};

const resolvePlaywrightChromiumExecutable = (
  packageRoot,
  env,
  spawnSyncImpl,
  nodeExecPath,
) => {
  const result = spawnSyncImpl(
    nodeExecPath,
    ["-e", "const { chromium } = require('playwright'); process.stdout.write(chromium.executablePath())"],
    {
      cwd: path.resolve(packageRoot),
      env,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    const stderr = String(result.stderr ?? "").trim();
    throw new Error(stderr || `Failed to resolve Playwright Chromium executable for ${packageRoot}`);
  }
  const executablePath = String(result.stdout ?? "").trim();
  if (!executablePath) {
    throw new Error(`Playwright did not report a Chromium executable for ${packageRoot}`);
  }
  return executablePath;
};

export const ensurePlaywrightBrowserInstall = (
  packageRoot,
  {
    env = process.env,
    spawnSyncImpl = spawnSync,
    existsSyncImpl = fs.existsSync,
    nodeExecPath = process.execPath,
  } = {},
) => {
  const resolvedPackageRoot = path.resolve(packageRoot);
  const executablePath = resolvePlaywrightChromiumExecutable(
    resolvedPackageRoot,
    env,
    spawnSyncImpl,
    nodeExecPath,
  );
  if (existsSyncImpl(executablePath)) {
    return executablePath;
  }

  const playwrightBin = requireLocalNodeBin(resolvedPackageRoot, "playwright");
  const installResult = spawnSyncImpl(playwrightBin, ["install", "chromium"], {
    cwd: resolvedPackageRoot,
    env,
    stdio: "inherit",
  });
  if (installResult.error) {
    throw installResult.error;
  }
  if (installResult.status !== 0) {
    throw new Error(
      `Playwright browser install failed for ${resolvedPackageRoot}; run '${resolvedPackageRoot}/node_modules/.bin/playwright install chromium'`,
    );
  }

  const installedExecutablePath = resolvePlaywrightChromiumExecutable(
    resolvedPackageRoot,
    env,
    spawnSyncImpl,
    nodeExecPath,
  );
  if (!existsSyncImpl(installedExecutablePath)) {
    throw new Error(
      `Playwright Chromium executable is still missing after install: ${installedExecutablePath}`,
    );
  }
  return installedExecutablePath;
};
