#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

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

  if (!skipWebBuild) {
    const pnpmCmd = process.platform === "win32" ? "pnpm.cmd" : "pnpm";
    runSync(pnpmCmd, ["-C", "apps/web", "build"], repoRoot, env);
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

try {
  main();
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}
