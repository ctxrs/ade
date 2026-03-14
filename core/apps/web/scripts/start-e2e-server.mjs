#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";

const parseBool = (value) => ["1", "true", "yes", "on"].includes(String(value ?? "").trim().toLowerCase());

const requireEnv = (key) => {
  const value = String(process.env[key] ?? "").trim();
  if (!value) {
    throw new Error(`Missing required environment variable: ${key}`);
  }
  return value;
};

const ensureSafeE2ETempDir = (dataDir) => {
  const resolved = path.resolve(dataDir);
  const tmpRoot = path.resolve(os.tmpdir());
  const relative = path.relative(tmpRoot, resolved);
  const isOutsideTmp = relative.startsWith("..") || path.isAbsolute(relative);
  if (isOutsideTmp) {
    throw new Error(`Refusing to delete non-temp e2e data dir: ${resolved}`);
  }
  const baseName = path.basename(resolved);
  if (!baseName.startsWith("ctx-e2e-")) {
    throw new Error(`Refusing to delete unexpected e2e data dir name: ${resolved}`);
  }
  return resolved;
};

const runSync = (command, args, cwd, env) => {
  const result = spawnSync(command, args, { cwd, env, stdio: "inherit" });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
};

const resolveCargoTargetDir = (repoRoot, env) => {
  const configured = String(env.CARGO_TARGET_DIR ?? "").trim();
  if (!configured) {
    return path.join(repoRoot, "target");
  }
  return path.isAbsolute(configured) ? configured : path.resolve(repoRoot, configured);
};

export const resolveE2EWebDistDir = (
  baseTmpDir = os.tmpdir(),
  pid = process.pid,
) => path.resolve(baseTmpDir, `ctx-e2e-web-dist-${pid}`);

export const resolveWebBuildArgs = (webDistDir) => [
  "-C",
  "apps/web",
  "exec",
  "vite",
  "build",
  "--outDir",
  webDistDir,
  "--emptyOutDir",
];

export const resolveServeWebDistDir = (repoRoot, env, skipWebBuild = false) => {
  const configured = String(env.CTX_WEB_DIST ?? "").trim();
  if (configured) {
    return path.isAbsolute(configured) ? configured : path.resolve(repoRoot, configured);
  }
  if (skipWebBuild) {
    return path.join(repoRoot, "apps", "web", "dist");
  }
  return resolveE2EWebDistDir();
};

export const shouldUseConfiguredCtxMcpCommand = (env) =>
  String(env.CTX_E2E_ALLOW_CONFIGURED_MCP_COMMAND ?? "").trim() === "1";

const ensureCtxMcpCommand = (repoRoot, env) => {
  const configured = String(env.CTX_MCP_COMMAND ?? "").trim();
  if (configured && shouldUseConfiguredCtxMcpCommand(env)) {
    return configured;
  }

  const cargoCmd = process.platform === "win32" ? "cargo.exe" : "cargo";
  runSync(cargoCmd, ["build", "-p", "ctx-mcp", "--bin", "ctx-mcp"], repoRoot, env);

  const binName = process.platform === "win32" ? "ctx-mcp.exe" : "ctx-mcp";
  const binaryPath = path.join(resolveCargoTargetDir(repoRoot, env), "debug", binName);
  if (!fs.existsSync(binaryPath)) {
    throw new Error(`ctx-mcp binary not found after build: ${binaryPath}`);
  }
  return binaryPath;
};

const main = () => {
  const repoRoot = process.cwd();
  const host = process.env.CTX_E2E_HOST ?? "127.0.0.1";
  const portText = requireEnv("CTX_E2E_PORT");
  const port = Number(portText);
  if (!Number.isInteger(port) || port <= 0 || port > 65535) {
    throw new Error(`Invalid CTX_E2E_PORT: ${portText}`);
  }

  const dataDir = ensureSafeE2ETempDir(requireEnv("CTX_E2E_DATA_DIR"));
  const authToken = requireEnv("CTX_E2E_AUTH_TOKEN");
  const skipWebBuild = parseBool(process.env.CTX_E2E_SKIP_WEB_BUILD);
  const env = { ...process.env };
  env.CTX_MCP_COMMAND = ensureCtxMcpCommand(repoRoot, env);
  const webDistDir = resolveServeWebDistDir(repoRoot, env, skipWebBuild);

  if (!skipWebBuild) {
    const pnpmCmd = process.platform === "win32" ? "pnpm.cmd" : "pnpm";
    fs.rmSync(webDistDir, { recursive: true, force: true });
    runSync(pnpmCmd, resolveWebBuildArgs(webDistDir), repoRoot, env);
  }

  fs.rmSync(dataDir, { recursive: true, force: true });
  fs.mkdirSync(dataDir, { recursive: true });
  fs.writeFileSync(path.join(dataDir, "daemon_auth.json"), JSON.stringify({ token: authToken }, null, 2));
  fs.writeFileSync(path.join(dataDir, "settings.json"), JSON.stringify({ execution: { mode: "host" } }, null, 2));

  const cargoCmd = process.platform === "win32" ? "cargo.exe" : "cargo";
  const child = spawn(
    cargoCmd,
    ["run", "-p", "ctx-http", "--bin", "ctx", "--", "serve", "--bind", `${host}:${port}`, "--data-dir", dataDir],
    {
      cwd: repoRoot,
      env: {
        ...env,
        CTX_EXECUTION_MODE: "host",
        CTX_SHOW_FAKE_PROVIDER: "1",
        CTX_STORAGE_BACKEND: "sqlite",
        CTX_WEB_DIST: webDistDir,
      },
      stdio: "inherit",
    },
  );

  const relaySignal = (signal) => {
    if (!child.killed) {
      child.kill(signal);
    }
  };

  process.on("SIGINT", () => relaySignal("SIGINT"));
  process.on("SIGTERM", () => relaySignal("SIGTERM"));
  process.on("SIGHUP", () => relaySignal("SIGHUP"));

  child.on("error", (err) => {
    console.error("Failed to launch ctx e2e server:", err);
    process.exit(1);
  });

  child.on("exit", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
      return;
    }
    process.exit(code ?? 1);
  });
};

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
}
