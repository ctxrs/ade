#!/usr/bin/env node

import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";

import { connectBrowser } from "./demo_ping_pong_playback.mjs";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const DESKTOP_ROOT = path.resolve(__dirname, "..");

function fail(message) {
  throw new Error(message);
}

function parseArgs(argv = process.argv.slice(2)) {
  const options = {
    appPath: "",
    timeoutMs: 60_000,
    pollMs: 1_000,
    artifactDir: "",
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = String(argv[index] || "").trim();
    switch (arg) {
      case "--app":
        options.appPath = path.resolve(String(argv[index + 1] || ""));
        index += 1;
        break;
      case "--timeout-ms":
        options.timeoutMs = Number.parseInt(String(argv[index + 1] || ""), 10);
        index += 1;
        break;
      case "--poll-ms":
        options.pollMs = Number.parseInt(String(argv[index + 1] || ""), 10);
        index += 1;
        break;
      case "--artifact-dir":
        options.artifactDir = path.resolve(String(argv[index + 1] || ""));
        index += 1;
        break;
      default:
        fail(`unknown argument: ${arg}`);
    }
  }
  if (!options.appPath) {
    fail("missing required --app <path>");
  }
  if (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0) {
    fail(`invalid --timeout-ms: ${String(options.timeoutMs)}`);
  }
  if (!Number.isFinite(options.pollMs) || options.pollMs <= 0) {
    fail(`invalid --poll-ms: ${String(options.pollMs)}`);
  }
  return options;
}

function pickUnusedPort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = typeof address === "object" && address ? address.port : 0;
      server.close((error) => {
        if (error) {
          reject(error);
          return;
        }
        if (!port) {
          reject(new Error("failed to allocate an unused TCP port"));
          return;
        }
        resolve(port);
      });
    });
  });
}

function waitForTcpPort(host, port, timeoutMs, label) {
  return new Promise((resolve, reject) => {
    const deadline = Date.now() + timeoutMs;
    const attempt = () => {
      const socket = net.createConnection({ host, port });
      socket.setTimeout(1_000);
      socket.once("connect", () => {
        socket.destroy();
        resolve();
      });
      socket.once("timeout", () => {
        socket.destroy();
      });
      socket.once("error", () => {
        socket.destroy();
      });
      socket.once("close", () => {
        if (Date.now() >= deadline) {
          reject(new Error(`${label} did not become ready within ${timeoutMs}ms`));
          return;
        }
        setTimeout(attempt, 250);
      });
    };
    attempt();
  });
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function normalizeText(value, limit = 2_000) {
  const text = typeof value === "string" ? value : String(value || "");
  return text.length > limit ? `${text.slice(0, limit)}...` : text;
}

async function readLaunchState(browser) {
  return browser.execute(() => ({
    href: window.location.href,
    pathname: window.location.pathname,
    readyState: document.readyState,
    title: document.title,
    hasRoot: Boolean(document.getElementById("root")),
    newWorkspaceVisible: /new workspace/i.test(String(document.body?.innerText || "")),
    bodyText: String(document.body?.innerText || "").slice(0, 2_000),
    bodyHtml: String(document.body?.innerHTML || "").slice(0, 4_000),
  }));
}

function spawnDriver({ port, nativePort, artifactDir }) {
  const driverLogPath = path.join(artifactDir, "tauri-driver.log");
  fs.mkdirSync(path.dirname(driverLogPath), { recursive: true });
  const driverLogFd = fs.openSync(driverLogPath, "a");
  const useXvfb = process.platform === "linux" && !String(process.env.DISPLAY || "").trim();
  const command = useXvfb ? "xvfb-run" : "pnpm";
  const portArgs = ["--port", String(port), "--native-port", String(nativePort)];
  const args = useXvfb
    ? ["-a", "pnpm", "exec", "tauri-driver", ...portArgs]
    : ["exec", "tauri-driver", ...portArgs];
  const proc = spawn(command, args, {
    cwd: DESKTOP_ROOT,
    stdio: ["ignore", driverLogFd, driverLogFd],
    env: {
      ...process.env,
      TAURI_DRIVER_PORT: String(port),
      TAURI_DRIVER_NATIVE_PORT: String(nativePort),
    },
  });
  return {
    proc,
    driverLogFd,
    driverLogPath,
  };
}

async function main() {
  if (process.platform !== "linux") {
    fail(`linux bundled launch smoke only supports Linux, got ${process.platform}`);
  }

  const options = parseArgs();
  if (!fs.existsSync(options.appPath)) {
    fail(`bundled app not found: ${options.appPath}`);
  }
  const artifactDir = options.artifactDir || fs.mkdtempSync(path.join(os.tmpdir(), "ctx-linux-launch-smoke-"));
  const driverPort = await pickUnusedPort();
  let nativeDriverPort = await pickUnusedPort();
  for (let attempt = 0; nativeDriverPort === driverPort && attempt < 5; attempt += 1) {
    nativeDriverPort = await pickUnusedPort();
  }
  if (nativeDriverPort === driverPort) {
    fail(`failed to allocate distinct tauri-driver native port: ${nativeDriverPort}`);
  }
  const { proc, driverLogFd, driverLogPath } = spawnDriver({
    port: driverPort,
    nativePort: nativeDriverPort,
    artifactDir,
  });

  let browser = null;
  try {
    proc.once("exit", (code, signal) => {
      if (!browser) {
        const tail = fs.existsSync(driverLogPath) ? normalizeText(fs.readFileSync(driverLogPath, "utf8"), 4_000) : "";
        process.stderr.write(
          `tauri-driver exited before browser session (code=${String(code)} signal=${String(signal)})\n${tail}\n`,
        );
      }
    });

    await waitForTcpPort("127.0.0.1", driverPort, 30_000, "tauri-driver");
    browser = await connectBrowser({ driverPort, appPath: options.appPath });

    const deadline = Date.now() + options.timeoutMs;
    let state = await readLaunchState(browser);
    while (Date.now() < deadline) {
      if (
        state.hasRoot
        && state.href !== "about:blank"
        && state.pathname !== "blank"
        && state.readyState === "complete"
      ) {
        break;
      }
      await sleep(options.pollMs);
      state = await readLaunchState(browser);
    }

    if (!state.hasRoot || state.href === "about:blank" || state.pathname === "blank") {
      fail(
        `bundled desktop launch smoke failed: ${JSON.stringify({
          href: state.href,
          pathname: state.pathname,
          readyState: state.readyState,
          hasRoot: state.hasRoot,
          newWorkspaceVisible: state.newWorkspaceVisible,
          bodyText: normalizeText(state.bodyText, 800),
        }, null, 2)}`,
      );
    }
    if (!state.newWorkspaceVisible) {
      fail(
        `bundled desktop launch smoke did not render launcher text: ${JSON.stringify({
          href: state.href,
          pathname: state.pathname,
          bodyText: normalizeText(state.bodyText, 800),
        }, null, 2)}`,
      );
    }

    process.stdout.write(`${JSON.stringify({
      ok: true,
      app_path: options.appPath,
      href: state.href,
      pathname: state.pathname,
      readyState: state.readyState,
      hasRoot: state.hasRoot,
      newWorkspaceVisible: state.newWorkspaceVisible,
      artifactDir,
      driverLogPath,
    }, null, 2)}\n`);
  } finally {
    if (browser) {
      try {
        await browser.deleteSession();
      } catch {
        // ignore cleanup failures
      }
    }
    if (proc && !proc.killed) {
      proc.kill("SIGTERM");
    }
    fs.closeSync(driverLogFd);
  }
}

main().catch((error) => {
  process.stderr.write(`${String(error?.stack || error)}\n`);
  process.exit(1);
});
