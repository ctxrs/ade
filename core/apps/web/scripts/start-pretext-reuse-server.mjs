#!/usr/bin/env node

import { spawn } from "node:child_process";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const volatileRoot =
  process.env.CTX_VOLATILE_TMPDIR
  ?? path.join(os.tmpdir(), "ctx-pretext-reuse");
const dataDir =
  process.env.CTX_E2E_DATA_DIR
  ?? path.join(volatileRoot, "ctx-e2e-pretext-reuse-data");
const tmpDir =
  process.env.CTX_E2E_TMPDIR
  ?? path.join(volatileRoot, "ctx-e2e-pretext-reuse-tmp");
const port = process.env.CTX_E2E_PORT ?? "4417";
const authToken = process.env.CTX_E2E_AUTH_TOKEN ?? "ctx-e2e-auth-token";

const child = spawn(process.execPath, [path.resolve(__dirname, "start-e2e-server.mjs")], {
  cwd: path.resolve(__dirname, "../../.."),
  env: {
    ...process.env,
    CTX_E2E_PORT: port,
    CTX_E2E_DATA_DIR: dataDir,
    CTX_E2E_TMPDIR: tmpDir,
    CTX_E2E_AUTH_TOKEN: authToken,
  },
  stdio: "inherit",
});

const relaySignal = (signal) => {
  if (!child.killed) {
    child.kill(signal);
  }
};

process.on("SIGINT", () => relaySignal("SIGINT"));
process.on("SIGTERM", () => relaySignal("SIGTERM"));
process.on("SIGHUP", () => relaySignal("SIGHUP"));

child.on("error", (error) => {
  console.error("Failed to start pretext reuse server:", error);
  process.exit(1);
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 1);
});
