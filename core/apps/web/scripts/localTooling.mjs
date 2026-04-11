import crypto from "node:crypto";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const pnpmCommand = process.platform === "win32" ? "pnpm.cmd" : "pnpm";
const lockedInstallArgs = ["install", "--frozen-lockfile"];
const installStampFile = ".ctx-locked-install.json";
const supportedPlaywrightBrowsers = ["chromium", "firefox", "webkit"];

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
const resolveLockfilePath = (packageRoot) => path.join(resolveLockedInstallRoot(packageRoot), "pnpm-lock.yaml");
const resolveInstallStampPath = (packageRoot) =>
  path.join(resolveLockedInstallRoot(packageRoot), "node_modules", installStampFile);

const resolveLockedInstallEnv = (env) => ({
  ...env,
  CI: env.CI ?? "1",
  npm_config_confirm_modules_purge: env.npm_config_confirm_modules_purge ?? "false",
});

export const lockedInstallHint = (packageRoot) =>
  `bash -lc "cd ${resolveLockedInstallRoot(packageRoot)} && pnpm ${lockedInstallArgs.join(" ")}"`;

const sha256File = (filePath) =>
  crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");

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
    existsSyncImpl = fs.existsSync,
    spawnSyncImpl = spawnSync,
    env = process.env,
    exitImpl = process.exit,
    requiredBins = [],
    writeFileSyncImpl = fs.writeFileSync,
  } = {},
) => {
  const workspaceRoot = resolveLockedInstallRoot(packageRoot);
  const lockfilePath = resolveLockfilePath(packageRoot);
  const stampPath = resolveInstallStampPath(packageRoot);
  const lockfileSha = existsSyncImpl(lockfilePath) ? sha256File(lockfilePath) : "";
  const hasRequiredBins = requiredBins.every((tool) => existsSyncImpl(resolveLocalNodeBin(packageRoot, tool)));
  if (
    lockfileSha
    && existsSyncImpl(path.join(workspaceRoot, "node_modules", ".modules.yaml"))
    && existsSyncImpl(stampPath)
    && hasRequiredBins
  ) {
    try {
      const stamp = JSON.parse(fs.readFileSync(stampPath, "utf8"));
      if (stamp?.lockfile_sha256 === lockfileSha) {
        return { reused: true, lockfileSha };
      }
    } catch {}
  }

  const result = spawnSyncImpl(pnpmCommand, lockedInstallArgs, {
    cwd: workspaceRoot,
    env: resolveLockedInstallEnv(env),
    stdio: "inherit",
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    exitImpl(result.status ?? 1);
  }
  fs.mkdirSync(path.dirname(stampPath), { recursive: true });
  writeFileSyncImpl(
    stampPath,
    `${JSON.stringify(
      {
        version: 1,
        lockfile_sha256: lockfileSha,
        updated_at: new Date().toISOString(),
      },
      null,
      2,
    )}\n`,
  );
  return { reused: false, lockfileSha };
};

export const requireLocalNodeBin = (packageRoot, tool) => {
  const binPath = resolveLocalNodeBin(packageRoot, tool);
  if (!fs.existsSync(binPath)) {
    throw new Error(`Missing local ${tool} binary for ${packageRoot}; run '${lockedInstallHint(packageRoot)}'`);
  }
  return binPath;
};

const resolvePlaywrightBrowserNames = (
  browsers,
  env,
) => {
  const rawEntries = Array.isArray(browsers)
    ? browsers
    : String(
      browsers
      ?? env.CTX_E2E_PLAYWRIGHT_BROWSERS
      ?? env.CTX_E2E_BROWSER
      ?? "webkit",
    ).split(",");
  const normalized = [];
  for (const entry of rawEntries) {
    const browser = String(entry).trim().toLowerCase();
    if (!browser) continue;
    if (!supportedPlaywrightBrowsers.includes(browser)) {
      throw new Error(
        `Unsupported Playwright browser '${browser}'. Expected one of: ${supportedPlaywrightBrowsers.join(", ")}`,
      );
    }
    if (!normalized.includes(browser)) {
      normalized.push(browser);
    }
  }
  return normalized.length > 0 ? normalized : ["webkit"];
};

const resolvePlaywrightBrowserExecutable = (
  packageRoot,
  browser,
  env,
  spawnSyncImpl,
  nodeExecPath,
) => {
  const result = spawnSyncImpl(
    nodeExecPath,
    [
      "-e",
      `const { ${browser} } = require('playwright'); process.stdout.write(${browser}.executablePath())`,
    ],
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
    throw new Error(`Playwright did not report a ${browser} executable for ${packageRoot}`);
  }
  return executablePath;
};

export const ensurePlaywrightBrowserInstall = (
  packageRoot,
  {
    browsers,
    env = process.env,
    spawnSyncImpl = spawnSync,
    existsSyncImpl = fs.existsSync,
    nodeExecPath = process.execPath,
  } = {},
) => {
  const resolvedPackageRoot = path.resolve(packageRoot);
  const requestedBrowsers = resolvePlaywrightBrowserNames(browsers, env);
  const executablePaths = Object.fromEntries(
    requestedBrowsers.map((browser) => [
      browser,
      resolvePlaywrightBrowserExecutable(
        resolvedPackageRoot,
        browser,
        env,
        spawnSyncImpl,
        nodeExecPath,
      ),
    ]),
  );
  const missingBrowsers = requestedBrowsers.filter(
    (browser) => !existsSyncImpl(executablePaths[browser]),
  );
  if (missingBrowsers.length === 0) {
    return executablePaths;
  }

  const playwrightBin = requireLocalNodeBin(resolvedPackageRoot, "playwright");
  const installResult = spawnSyncImpl(playwrightBin, ["install", ...missingBrowsers], {
    cwd: resolvedPackageRoot,
    env,
    stdio: "inherit",
  });
  if (installResult.error) {
    throw installResult.error;
  }
  if (installResult.status !== 0) {
    throw new Error(
      `Playwright browser install failed for ${resolvedPackageRoot}; run '${resolvedPackageRoot}/node_modules/.bin/playwright install ${missingBrowsers.join(" ")}'`,
    );
  }

  const installedExecutablePaths = Object.fromEntries(
    requestedBrowsers.map((browser) => [
      browser,
      resolvePlaywrightBrowserExecutable(
        resolvedPackageRoot,
        browser,
        env,
        spawnSyncImpl,
        nodeExecPath,
      ),
    ]),
  );
  const stillMissing = requestedBrowsers.filter(
    (browser) => !existsSyncImpl(installedExecutablePaths[browser]),
  );
  if (stillMissing.length > 0) {
    throw new Error(
      `Playwright browser executable is still missing after install: ${stillMissing.join(", ")}`,
    );
  }
  return installedExecutablePaths;
};
