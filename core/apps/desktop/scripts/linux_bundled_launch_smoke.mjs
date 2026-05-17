#!/usr/bin/env node

import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { spawn, spawnSync } from "node:child_process";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const DESKTOP_ROOT = path.resolve(__dirname, "..");
const CORE_ROOT = path.resolve(DESKTOP_ROOT, "../..");
const REPO_ROOT = path.resolve(CORE_ROOT, "..");
const require = createRequire(import.meta.url);
const {
  buildLinuxAppDirLaunchEnv,
  createLinuxAppDirLaunchWrapper,
  resolveLinuxAppDirFromPath,
} = require("../automation/helpers/linux_appdir_launch_env.cjs");

const SESSION_START_ATTEMPTS = 2;
const SESSION_START_TIMEOUT_MS = positiveIntegerEnv(
  "CTX_LINUX_BUNDLED_LAUNCH_SESSION_TIMEOUT_MS",
  120_000,
);
const RETRYABLE_STARTUP_SESSION_FAILURE =
  /(UND_ERR_HEADERS_TIMEOUT|Failed to create a session|WebDriver session creation timed out|IncompleteMessage|socket hang up|ECONNRESET|ECONNREFUSED)/i;

const DESKTOP_APP_LAUNCH_ENV_KEYS = [
  "APPDIR",
  "APPIMAGE",
  "APPIMAGE_EXTRACT_AND_RUN",
  "ARGV0",
  "CTX_APPIMAGE_PATH",
  "CTX_BUNDLE_DIR",
  "CTX_DESKTOP_DAEMON_DATA_DIR",
  "CTX_DESKTOP_SSH_NO_START_REMOTE",
  "CTX_DESKTOP_SSH_START_REMOTE",
  "DISPLAY",
  "HOME",
  "TMP",
  "TEMP",
  "TMPDIR",
  "XDG_CACHE_HOME",
  "XDG_CONFIG_HOME",
  "XDG_DATA_HOME",
  "XDG_RUNTIME_DIR",
];

function fail(message) {
  throw new Error(message);
}

function positiveIntegerEnv(name, fallback) {
  const rawValue = String(process.env[name] || "").trim();
  if (!rawValue) return fallback;
  const parsed = Number.parseInt(rawValue, 10);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    throw new Error(`${name} must be a positive integer, got ${rawValue}`);
  }
  return parsed;
}

function resolveConfiguredPath(rawValue) {
  const configured = String(rawValue || "").trim();
  if (!configured) return "";
  return path.isAbsolute(configured) ? configured : path.resolve(configured);
}

function isTruthyEnv(value) {
  return /^(1|true|yes)$/i.test(String(value || "").trim());
}

function parseArgs(argv = process.argv.slice(2)) {
  const options = {
    appPath: "",
    sessionTimeoutMs: SESSION_START_TIMEOUT_MS,
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
      case "--session-timeout-ms":
        options.sessionTimeoutMs = Number.parseInt(String(argv[index + 1] || ""), 10);
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
  if (!Number.isFinite(options.sessionTimeoutMs) || options.sessionTimeoutMs <= 0) {
    fail(`invalid --session-timeout-ms: ${String(options.sessionTimeoutMs)}`);
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

function errorText(error) {
  return String(error?.stack || error?.message || error || "");
}

function isRetryableStartupSessionFailure(error) {
  return RETRYABLE_STARTUP_SESSION_FAILURE.test(errorText(error));
}

async function withTimeout(promise, timeoutMs, label) {
  let timer = null;
  let timedOut = false;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => {
      timedOut = true;
      reject(new Error(`${label} timed out after ${timeoutMs}ms`));
    }, timeoutMs);
  });
  try {
    return await Promise.race([promise, timeout]);
  } finally {
    if (timer) {
      clearTimeout(timer);
    }
    if (timedOut && promise && typeof promise.catch === "function") {
      promise.catch(() => {});
    }
  }
}

function pickEnv(env, keys) {
  return Object.fromEntries(
    keys
      .map((key) => [key, String(env[key] || "")])
      .filter(([, value]) => value.trim().length > 0),
  );
}

function describePath(targetPath) {
  if (!targetPath) return { path: "", exists: false };
  try {
    const stat = fs.statSync(targetPath);
    return {
      path: targetPath,
      exists: true,
      mode: `0${(stat.mode & 0o777).toString(8)}`,
      size: stat.size,
      isFile: stat.isFile(),
      isDirectory: stat.isDirectory(),
    };
  } catch (error) {
    return {
      path: targetPath,
      exists: false,
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

function writeProcessSnapshot(filePath) {
  const result = spawnSync("ps", ["-eo", "pid,ppid,stat,etime,command"], {
    encoding: "utf8",
  });
  fs.writeFileSync(
    filePath,
    result.status === 0
      ? String(result.stdout || "")
      : `ps failed status=${String(result.status)} stderr=${String(result.stderr || "")}\n`,
  );
}

function readOptionalFileTail(filePath, limit = 4_000) {
  try {
    if (!filePath || !fs.existsSync(filePath)) {
      return "";
    }
    return normalizeText(fs.readFileSync(filePath, "utf8"), limit);
  } catch (error) {
    return `failed to read ${filePath}: ${error instanceof Error ? error.message : String(error)}`;
  }
}

function writeStartupFailureDiagnostics({
  appLaunchEnv,
  applicationPath,
  artifactDir,
  driverPort,
  error,
  nativeDriverPort,
  requestedAppPath,
}) {
  fs.mkdirSync(artifactDir, { recursive: true });
  const appLaunchLog = String(appLaunchEnv.CTX_AUTOMATION_APP_LAUNCH_LOG || "");
  const payload = {
    app_launch_log: appLaunchLog,
    app_launch_log_exists: Boolean(appLaunchLog && fs.existsSync(appLaunchLog)),
    app_launch_log_tail: readOptionalFileTail(appLaunchLog),
    application_path: applicationPath,
    driver_port: driverPort,
    error: normalizeText(errorText(error), 4_000),
    native_driver_port: nativeDriverPort,
    requested_app_path: requestedAppPath,
  };
  fs.writeFileSync(
    path.join(artifactDir, "startup-failure-diagnostics.json"),
    `${JSON.stringify(payload, null, 2)}\n`,
  );
}

function writeLaunchDiagnostics({
  artifactDir,
  requestedAppPath,
  applicationPath,
  appLaunchEnv,
  driverPort,
  nativeDriverPort,
}) {
  fs.mkdirSync(artifactDir, { recursive: true });
  const appDir = resolveLinuxAppDirFromPath({
    appPath: requestedAppPath,
    env: {
      ...process.env,
      ...appLaunchEnv,
    },
  });
  const targets = appDir
    ? [
        path.join(appDir, "AppRun"),
        path.join(appDir, "AppRun.wrapped"),
        path.join(appDir, "usr", "bin", "ctx"),
        path.join(appDir, "apprun-hooks", "linuxdeploy-plugin-gtk.sh"),
      ]
    : [requestedAppPath];
  const envKeys = [
    "APPDIR",
    "APPIMAGE",
    "APPIMAGE_EXTRACT_AND_RUN",
    "ARGV0",
    "CTX_APPIMAGE_PATH",
    "CTX_AUTOMATION_APP_LAUNCH_LOG",
    "CTX_BUNDLE_DIR",
    "CTX_DESKTOP_DAEMON_DATA_DIR",
    "CTX_DESKTOP_SSH_NO_START_REMOTE",
    "CTX_DESKTOP_SSH_START_REMOTE",
    "DISPLAY",
    "GDK_BACKEND",
    "GIO_EXTRA_MODULES",
    "GSETTINGS_SCHEMA_DIR",
    "GTK_DATA_PREFIX",
    "GTK_EXE_PREFIX",
    "GTK_IM_MODULE_FILE",
    "GTK_PATH",
    "HOME",
    "LD_LIBRARY_PATH",
    "PATH",
    "TAURI_WEBVIEW_AUTOMATION",
    "TMPDIR",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_DIRS",
    "XDG_DATA_HOME",
    "XDG_RUNTIME_DIR",
  ];
  fs.writeFileSync(
    path.join(artifactDir, "launch-env.json"),
    `${JSON.stringify({
      requestedAppPath,
      applicationPath,
      appDir,
      driverPort,
      nativeDriverPort,
      env: pickEnv({ ...process.env, ...appLaunchEnv }, envKeys),
      targets: targets.map(describePath),
    }, null, 2)}\n`,
  );
  writeProcessSnapshot(path.join(artifactDir, "processes-before-session.log"));
}

function buildDesktopAppLaunchEnv({ appPath, artifactDir }) {
  const env = {
    TAURI_WEBVIEW_AUTOMATION: "true",
  };
  for (const key of DESKTOP_APP_LAUNCH_ENV_KEYS) {
    const value = String(process.env[key] || "").trim();
    if (value) {
      env[key] = value;
    }
  }
  Object.assign(
    env,
    buildLinuxAppDirLaunchEnv({
      appPath,
      env: {
        ...process.env,
        ...env,
      },
    }),
  );
  if (
    !String(env.CTX_DESKTOP_SSH_NO_START_REMOTE || "").trim()
    && !String(env.CTX_DESKTOP_SSH_START_REMOTE || "").trim()
  ) {
    env.CTX_DESKTOP_SSH_NO_START_REMOTE = "1";
    env.CTX_DESKTOP_SSH_START_REMOTE = "0";
  }
  if (isTruthyEnv(process.env.CTX_AUTOMATION_SHIPPED_APP)) {
    const bundleDir = resolveConfiguredPath(process.env.CTX_AUTOMATION_SHIPPED_APP_BUNDLES_DIR);
    if (bundleDir && !String(env.CTX_BUNDLE_DIR || "").trim()) {
      env.CTX_BUNDLE_DIR = bundleDir;
    }
    const daemonDataDir = resolveConfiguredPath(
      process.env.CTX_AUTOMATION_SHIPPED_APP_DAEMON_DATA_DIR,
    );
    if (daemonDataDir && !String(env.CTX_DESKTOP_DAEMON_DATA_DIR || "").trim()) {
      env.CTX_DESKTOP_DAEMON_DATA_DIR = daemonDataDir;
    }
  }
  const appLaunchLog = String(process.env.CTX_AUTOMATION_APP_LAUNCH_LOG || "").trim()
    || path.join(artifactDir, "app-launch.log");
  env.CTX_AUTOMATION_APP_LAUNCH_LOG = appLaunchLog;
  return { appLaunchEnv: env, appLaunchLog };
}

function requireWorkspacePackage(specifier) {
  try {
    const resolved = require.resolve(specifier, {
      paths: [
        CORE_ROOT,
        DESKTOP_ROOT,
        REPO_ROOT,
      ],
    });
    return require(resolved);
  } catch (error) {
    throw new Error(
      `missing workspace dependency "${specifier}". Run \`pnpm -C core install --frozen-lockfile\` before running the Linux bundled launch smoke.`,
      { cause: error },
    );
  }
}

async function connectBrowser({ driverPort, appPath, sessionTimeoutMs }) {
  const { remote } = requireWorkspacePackage("webdriverio");
  return remote({
    hostname: "127.0.0.1",
    port: driverPort,
    path: "/",
    connectionRetryCount: 0,
    connectionRetryTimeout: sessionTimeoutMs,
    capabilities: {
      "tauri:options": {
        application: appPath,
      },
      timeouts: {
        script: 180000,
        pageLoad: 300000,
        implicit: 0,
      },
    },
    automationProtocol: "webdriver",
    logLevel: "error",
  });
}

async function connectBrowserWithTimeout({ driverPort, appPath, sessionTimeoutMs }) {
  return withTimeout(
    connectBrowser({ driverPort, appPath, sessionTimeoutMs }),
    sessionTimeoutMs,
    "WebDriver session creation",
  );
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

function spawnDriver({ port, nativePort, artifactDir, appLaunchEnv }) {
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
    detached: process.platform !== "win32",
    env: {
      ...process.env,
      ...appLaunchEnv,
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

async function terminateDriverProcess(proc) {
  if (!proc || !proc.pid) return;
  const killTarget = process.platform === "win32" ? proc.pid : -proc.pid;
  const sendSignal = (signal) => {
    try {
      process.kill(killTarget, signal);
    } catch (error) {
      if (!error || error.code !== "ESRCH") {
        throw error;
      }
    }
  };
  sendSignal("SIGTERM");
  await sleep(1000);
  sendSignal("SIGKILL");
}

async function runLaunchAttempt({ options, artifactDir, appLaunchEnv, attempt, totalAttempts }) {
  // Linux WebKitWebDriver starts the application from the requested executable
  // path. Use the same AppDir launcher wrapper contract as release automation so
  // the child process gets the exact AppDir/AppImage env even when WebKit does
  // not preserve the full tauri-driver environment.
  const applicationPath = createLinuxAppDirLaunchWrapper({
    appPath: options.appPath,
    env: appLaunchEnv,
    wrapperDir: path.join(artifactDir, "desktop-app-launchers"),
  });
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
    appLaunchEnv,
  });
  fs.writeSync(driverLogFd, `\n--- linux bundled launch smoke attempt ${attempt}/${totalAttempts} ---\n`);
  writeLaunchDiagnostics({
    artifactDir,
    requestedAppPath: options.appPath,
    applicationPath,
    appLaunchEnv,
    driverPort,
    nativeDriverPort,
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
    browser = await connectBrowserWithTimeout({
      driverPort,
      appPath: applicationPath,
      sessionTimeoutMs: options.sessionTimeoutMs,
    });

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
    return { state, driverLogPath };
  } catch (error) {
    if (!browser && isRetryableStartupSessionFailure(error)) {
      error.retryableStartupSessionFailure = true;
    }
    if (!browser) {
      writeStartupFailureDiagnostics({
        appLaunchEnv,
        applicationPath,
        artifactDir,
        driverPort,
        error,
        nativeDriverPort,
        requestedAppPath: options.appPath,
      });
      process.stderr.write(
        `linux bundled launch smoke startup diagnostics written: ${path.join(artifactDir, "startup-failure-diagnostics.json")}\n`,
      );
    }
    throw error;
  } finally {
    writeProcessSnapshot(path.join(artifactDir, "processes-after-session.log"));
    if (browser) {
      try {
        await browser.deleteSession();
      } catch {
        // ignore cleanup failures
      }
    }
    await terminateDriverProcess(proc);
    writeProcessSnapshot(path.join(artifactDir, "processes-after-cleanup.log"));
    fs.closeSync(driverLogFd);
  }
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
  const { appLaunchEnv, appLaunchLog } = buildDesktopAppLaunchEnv({
    appPath: options.appPath,
    artifactDir,
  });

  let lastError = null;
  for (let attempt = 1; attempt <= SESSION_START_ATTEMPTS; attempt += 1) {
    try {
      const { state, driverLogPath } = await runLaunchAttempt({
        options,
        artifactDir,
        appLaunchEnv,
        attempt,
        totalAttempts: SESSION_START_ATTEMPTS,
      });

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
        appLaunchLog,
      }, null, 2)}\n`);
      return;
    } catch (error) {
      lastError = error;
      if (
        attempt < SESSION_START_ATTEMPTS
        && error?.retryableStartupSessionFailure === true
      ) {
        process.stderr.write(
          `linux bundled launch smoke retrying startup-only WebDriver session failure after attempt ${attempt}/${SESSION_START_ATTEMPTS}: ${normalizeText(errorText(error), 1_000)}\n`,
        );
        continue;
      }
      throw error;
    }
  }
  throw lastError || new Error("linux bundled launch smoke exhausted startup attempts");
}

main().catch((error) => {
  process.stderr.write(`${String(error?.stack || error)}\n`);
  process.exit(1);
});
