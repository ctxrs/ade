const path = require("path");
const fs = require("fs");
const { spawnSync, spawn } = require("child_process");
const os = require("os");

const { waitTestRunnerBackendReady } = require("@crabnebula/test-runner-backend");
const { waitTauriDriverReady } = require("@crabnebula/tauri-driver");

const ROOT = path.resolve(__dirname, "..");
const CORE_ROOT = path.resolve(ROOT, "..", "..");
const APP_PATH = process.env.CTX_DESKTOP_APP_PATH ||
  path.resolve(ROOT, "src-tauri/target/debug/bundle/macos/ctx.app");
const WORKSPACE_PATH = process.env.CTX_AUTOMATION_WORKSPACE_PATH ||
  "/Users/example-user/code/ctx-monorepo";

const TAURI_DRIVER_PORT = Number(process.env.TAURI_DRIVER_PORT || 4444);
const TEST_BACKEND_PORT = Number(process.env.TAURI_TEST_BACKEND_PORT || 3000);

const CTX_BIN = process.env.CTX_AUTOMATION_CTX_BIN ||
  path.resolve(ROOT, "src-tauri/bin/ctx");

const USE_EXTERNAL_DAEMON = !["0", "false", "no"].includes(
  String(process.env.CTX_AUTOMATION_USE_EXTERNAL_DAEMON || "1").trim().toLowerCase(),
);

let daemonProcess = null;
let daemonDataDir = null;
let daemonPort = null;
let daemonLogPath = null;

const killExistingAppProcesses = () => {
  if (!fs.existsSync(APP_PATH)) return;
  const appBin = path.resolve(APP_PATH, "Contents", "MacOS", "ctx");
  const appResBinPrefix = path.resolve(APP_PATH, "Contents", "Resources", "bin");
  const out = spawnSync("ps", ["-Ao", "pid=,command="], { encoding: "utf8" });
  if (out.status !== 0) return;
  const lines = String(out.stdout || "").split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
  const pids = [];
  for (const line of lines) {
    const m = line.match(/^(\d+)\s+(.*)$/);
    if (!m) continue;
    const pid = Number(m[1]);
    const cmd = m[2] || "";
    if (!pid || !cmd) continue;
    if (cmd.startsWith(appBin) || cmd.includes(appBin)) {
      pids.push(pid);
      continue;
    }
    // If the app previously spawned an internal daemon, kill it too (scoped to this app bundle).
    if (cmd.startsWith(appResBinPrefix) && /\bserve\b/.test(cmd)) {
      pids.push(pid);
    }
  }
  if (!pids.length) return;
  // Best-effort; ignore errors.
  spawnSync("kill", ["-9", ...pids.map(String)], { stdio: "ignore" });
};

const pickUnusedPort = () => {
  const net = require("net");
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.listen(0, "127.0.0.1", () => {
      const port = srv.address().port;
      srv.close(() => resolve(port));
    });
    srv.on("error", reject);
  });
};

const readJson = (p) => JSON.parse(fs.readFileSync(p, "utf8"));

const waitForHealth = async (baseUrl, timeoutMs) => {
  const started = Date.now();
  // Use curl when available to keep behavior close to shell scripts in this repo.
  while (Date.now() - started < timeoutMs) {
    const res = spawnSync("curl", ["-fsS", `${baseUrl}/api/health`], { stdio: "ignore" });
    if (res.status === 0) return;
    await new Promise((r) => setTimeout(r, 200));
  }
  let tailInfo = "";
  if (daemonLogPath && fs.existsSync(daemonLogPath)) {
    try {
      const log = fs.readFileSync(daemonLogPath, "utf8");
      const tail = log.split(/\r?\n/).slice(-200).join("\n");
      if (tail.trim()) {
        tailInfo = `\n--- daemon.log tail ---\n${tail}`;
      }
    } catch {
      // ignore
    }
  }
  throw new Error(`daemon did not become healthy in time: ${baseUrl}${tailInfo}`);
};

const startExternalDaemon = async () => {
  if (!fs.existsSync(CTX_BIN)) {
    throw new Error(`ctx binary not found at ${CTX_BIN} (run pnpm -C core desktop:prep)`);
  }
  daemonPort = await pickUnusedPort();
  daemonDataDir = fs.mkdtempSync(path.join(os.tmpdir(), `ctx-desktop-e2e-${daemonPort}-`));
  daemonLogPath = path.join(daemonDataDir, "daemon.log");
  const baseUrl = `http://127.0.0.1:${daemonPort}`;

  const logFd = fs.openSync(daemonLogPath, "a");
  daemonProcess = spawn(
    CTX_BIN,
    ["serve", "--bind", `127.0.0.1:${daemonPort}`, "--data-dir", daemonDataDir],
    {
      stdio: ["ignore", logFd, logFd],
      env: { ...process.env },
    },
  );

  await waitForHealth(baseUrl, 20000);
  const authPath = path.join(daemonDataDir, "daemon_auth.json");
  if (!fs.existsSync(authPath)) {
    throw new Error(`daemon_auth.json not found at ${authPath}`);
  }
  const auth = readJson(authPath);
  if (!auth.token) {
    throw new Error("daemon_auth.json missing token");
  }
  process.env.CTX_DESKTOP_DAEMON_URL = baseUrl;
  process.env.CTX_DESKTOP_DAEMON_TOKEN = auth.token;
};

const buildAppIfMissing = () => {
  if (process.env.CTX_AUTOMATION_SKIP_APP_BUILD) return;
  const result = spawnSync(
    "pnpm",
    // Build a macOS bundle so the driver can launch `ctx.app` at APP_PATH.
    ["tauri", "build", "--debug", "--", "--features", "automation"],
    { stdio: "inherit", cwd: path.resolve(ROOT, "src-tauri"), shell: true },
  );
  if (result.status !== 0) {
    throw new Error("Failed to build the Tauri app for automation.");
  }
};

let backendProcess = null;
let driverProcess = null;

exports.config = {
  runner: "local",
  framework: "mocha",
  reporters: ["spec"],
  // Run sequentially (Tauri + external daemon are shared global resources).
  specs: [path.resolve(__dirname, "specs/workspace-wizard.spec.cjs")],
  mochaOpts: {
    timeout: 120000,
  },
  logLevel: "info",
  maxInstances: 1,
  capabilities: [
    {
      browserName: "tauri",
      maxInstances: 1,
      "tauri:options": {
        application: APP_PATH,
      },
    },
  ],
  port: TAURI_DRIVER_PORT,
  path: "/",
  automationProtocol: "webdriver",
  beforeSession: () => {
    process.env.CTX_AUTOMATION_WORKSPACE_PATH = WORKSPACE_PATH;
  },
  onPrepare: async () => {
    if (process.platform === "darwin" && !process.env.CN_API_KEY) {
      throw new Error("CN_API_KEY is required for CrabNebula WebDriver on macOS.");
    }
    // Ensure we don't hit the single-instance path (which can forward to a stale app instance
    // without the automation plugin enabled).
    killExistingAppProcesses();

    // Container-mode provider smoke needs a fully-bundled release-style resource set
    // (Linux provider binaries + harness image tars). Keep the app build in debug mode
    // for the automation plugin, but sync release resources.
    const prepRelease = spawnSync("pnpm", ["-C", CORE_ROOT, "desktop:prep:release"], {
      stdio: "inherit",
      cwd: ROOT,
      shell: true,
    });
    if (prepRelease.status !== 0) {
      throw new Error("pnpm -C core desktop:prep:release failed");
    }

    // Launch the app directly into the wizard route to reduce test flakiness.
    process.env.CTX_DESKTOP_START_PATH = "/workspace-setup";
    process.env.CTX_SEED_CODEX_AUTH_FROM_HOST = process.env.CTX_SEED_CODEX_AUTH_FROM_HOST || "1";
    // For the shared Ashburn host, never attempt to start/restart the remote daemon from tests.
    process.env.CTX_DESKTOP_SSH_NO_START_REMOTE = "1";
    if (USE_EXTERNAL_DAEMON) {
      await startExternalDaemon();
    } else {
      // Ensure we validate the real launcher path: the app must spawn/connect its own daemon.
      delete process.env.CTX_DESKTOP_DAEMON_URL;
      delete process.env.CTX_DESKTOP_DAEMON_TOKEN;
    }

    buildAppIfMissing();

    backendProcess = spawn("pnpm", ["exec", "test-runner-backend"], {
      stdio: "inherit",
      cwd: ROOT,
      env: {
        ...process.env,
        TEST_RUNNER_BACKEND_PORT: String(TEST_BACKEND_PORT),
      },
    });
    await waitTestRunnerBackendReady();

    driverProcess = spawn("pnpm", ["exec", "tauri-driver"], {
      stdio: "inherit",
      cwd: ROOT,
      env: {
        ...process.env,
        TAURI_DRIVER_PORT: String(TAURI_DRIVER_PORT),
        REMOTE_WEBDRIVER_URL: `http://127.0.0.1:${TEST_BACKEND_PORT}`,
      },
    });
    await waitTauriDriverReady();
  },
  onComplete: () => {
    killExistingAppProcesses();
    if (driverProcess) {
      driverProcess.kill();
      driverProcess = null;
    }
    if (backendProcess) {
      backendProcess.kill();
      backendProcess = null;
    }
    if (daemonProcess) {
      daemonProcess.kill();
      daemonProcess = null;
    }
    if (daemonDataDir) {
      try {
        fs.rmSync(daemonDataDir, { recursive: true, force: true });
      } catch {
        // ignore
      }
      daemonDataDir = null;
      daemonLogPath = null;
    }
  },
};
