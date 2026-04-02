import { existsSync, mkdirSync } from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

export const GOOGLE_CHROME_EXECUTABLE = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
export const PLAYWRIGHT_CORE_VERSION = "1.57.0";
export const REPO_ROOT = path.resolve(path.dirname(new URL(import.meta.url).pathname), "../../../..");

export function pickUnusedPortSync(fallback) {
  const script = [
    "const net = require('node:net');",
    "const server = net.createServer();",
    "server.on('error', () => process.exit(2));",
    "server.listen(0, '127.0.0.1', () => {",
    "  const addr = server.address();",
    "  const port = addr && typeof addr === 'object' ? addr.port : 0;",
    "  server.close(() => {",
    "    if (!port) process.exit(3);",
    "    process.stdout.write(String(port));",
    "  });",
    "});",
  ].join("\n");
  const result = spawnSync(process.execPath, ["-e", script], { encoding: "utf8" });
  if (result.status !== 0) {
    return fallback;
  }
  const parsed = Number.parseInt(String(result.stdout || "").trim(), 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

export function runChecked(command, args, options = {}) {
  const result = spawnSync(command, args, {
    stdio: "inherit",
    ...options,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with status ${result.status}`);
  }
}

export function ensurePlaywrightCoreRuntime(runtimeDir) {
  const packageMarker = path.join(runtimeDir, "node_modules", "playwright-core", "package.json");
  if (existsSync(packageMarker)) {
    return;
  }

  mkdirSync(runtimeDir, { recursive: true });
  runChecked("npm", [
    "install",
    "--no-save",
    "--prefix", runtimeDir,
    `playwright-core@${PLAYWRIGHT_CORE_VERSION}`,
  ], { cwd: REPO_ROOT });
}
