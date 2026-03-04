const path = require("path");
const fs = require("fs");
const { spawnSync, spawn } = require("child_process");
const os = require("os");

const { waitTestRunnerBackendReady } = require("@crabnebula/test-runner-backend");
const { waitTauriDriverReady } = require("@crabnebula/tauri-driver");

const ROOT = path.resolve(__dirname, "..");
const CORE_ROOT = path.resolve(ROOT, "..", "..");
const defaultAppPath = (() => {
  if (process.platform === "darwin") {
    return path.resolve(ROOT, "src-tauri/target/debug/bundle/macos/ctx.app");
  }
  if (process.platform === "linux") {
    return path.resolve(ROOT, "src-tauri/target/debug/ctx");
  }
  if (process.platform === "win32") {
    return path.resolve(ROOT, "src-tauri/target/debug/ctx.exe");
  }
  return path.resolve(ROOT, "src-tauri/target/debug/ctx");
})();
const APP_PATH = process.env.CTX_DESKTOP_APP_PATH || defaultAppPath;
const BUNDLES_DIR = path.resolve(ROOT, "src-tauri/bundles");
const WORKSPACE_PATH = [
  String(process.env.CTX_AUTOMATION_WORKSPACE_PATH || "").trim(),
  String(process.env.GITHUB_WORKSPACE || "").trim(),
  path.resolve(CORE_ROOT, ".."),
  CORE_ROOT,
].find((candidate) => candidate && fs.existsSync(candidate));

const TAURI_DRIVER_PORT = Number(process.env.TAURI_DRIVER_PORT || 4444);
const TEST_BACKEND_PORT = Number(process.env.TAURI_TEST_BACKEND_PORT || 3000);

const CTX_BIN = process.env.CTX_AUTOMATION_CTX_BIN ||
  path.resolve(ROOT, "src-tauri/bin/ctx");

const USE_EXTERNAL_DAEMON = !["0", "false", "no"].includes(
  String(process.env.CTX_AUTOMATION_USE_EXTERNAL_DAEMON || "0").trim().toLowerCase(),
);
const SSH_NO_START_REMOTE = !["0", "false", "no"].includes(
  String(process.env.CTX_AUTOMATION_SSH_NO_START_REMOTE || "1").trim().toLowerCase(),
);
const SKIP_PREP_RELEASE = ["1", "true", "yes"].includes(
  String(process.env.CTX_AUTOMATION_SKIP_DESKTOP_PREP_RELEASE || "0").trim().toLowerCase(),
);
const SKIP_APP_BUILD = ["1", "true", "yes"].includes(
  String(process.env.CTX_AUTOMATION_SKIP_APP_BUILD || "0").trim().toLowerCase(),
);
const REMOTE_CTX_BIN = String(process.env.CTX_AUTOMATION_REMOTE_CTX_BIN || "").trim();
const REMOTE_SSH_KEY_PATH = String(
  process.env.CTX_AUTOMATION_REMOTE_SSH_KEY_PATH || process.env.CTX_UPDATER_E2E_SSH_KEY_PATH || "",
).trim();
const SKIP_REMOTE_CTX_PROVISION = ["1", "true", "yes"].includes(
  String(process.env.CTX_AUTOMATION_SKIP_REMOTE_CTX_PROVISION || "0").trim().toLowerCase(),
);
const WDIO_LOG_LEVEL = String(process.env.CTX_AUTOMATION_WDIO_LOG_LEVEL || "info").trim() || "info";
const parsePositiveInt = (raw, fallback) => {
  const n = Number.parseInt(String(raw ?? ""), 10);
  if (!Number.isFinite(n) || n <= 0) return fallback;
  return n;
};
const MOCHA_TIMEOUT_MS = parsePositiveInt(process.env.CTX_AUTOMATION_MOCHA_TIMEOUT_MS || "300000", 300000);
const SCENARIO_FILTER = String(process.env.CTX_AUTOMATION_SCENARIOS || "")
  .split(",")
  .map((token) => token.trim().toLowerCase())
  .filter(Boolean);
const CONTAINER_SCENARIO_TOKENS = new Set([
  "local",
  "container",
  "host-mounted",
  "disk-isolated",
  "provider",
  "remote-container",
  "local-clone-disk-isolated",
  "local-new-host-mounted",
  "local-new-disk-isolated",
  "local-codex-smoke",
  "remote-container-import",
]);
const RUNS_CONTAINER_SCENARIOS = SCENARIO_FILTER.length === 0
  || SCENARIO_FILTER.some((token) => CONTAINER_SCENARIO_TOKENS.has(token));

let daemonProcess = null;
let daemonDataDir = null;
let daemonPort = null;
let daemonLogPath = null;
let internalDaemonDataDir = null;

const killProcesses = (matcher) => {
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
    if (matcher(pid, cmd)) pids.push(pid);
  }
  if (!pids.length) return;
  spawnSync("kill", ["-9", ...pids.map(String)], { stdio: "ignore" });
};

const killExistingAppProcesses = () => {
  if (!fs.existsSync(APP_PATH)) return;
  const appPathStat = fs.statSync(APP_PATH);
  const appBin = appPathStat.isDirectory()
    ? path.resolve(APP_PATH, "Contents", "MacOS", "ctx")
    : APP_PATH;
  const appResBinPrefix = appPathStat.isDirectory()
    ? path.resolve(APP_PATH, "Contents", "Resources", "bin")
    : path.resolve(path.dirname(APP_PATH), "bin");
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

const killStaleAutomationHelpers = () => {
  // Clear stale tauri-driver/backend processes from previous crashed runs.
  killProcesses((_pid, cmd) =>
    /\btauri-driver\b/.test(cmd) ||
    /\btest-runner-backend\b/.test(cmd) ||
    /\bWebKitWebDriver\b/.test(cmd),
  );
};

const stopSystemdScope = (scopeName) => {
  spawnSync("systemctl", ["--user", "stop", scopeName], { stdio: "ignore" });
  spawnSync("systemctl", ["--user", "reset-failed", scopeName], { stdio: "ignore" });
};

const stopStaleSystemdScope = () => {
  if (process.platform !== "linux") return;
  stopSystemdScope("ctx-daemon.scope");
  const list = spawnSync(
    "systemctl",
    ["--user", "list-units", "--all", "--plain", "--no-legend", "ctx-daemon-*.scope"],
    { encoding: "utf8" },
  );
  if (list.status !== 0) return;
  const scopes = String(list.stdout || "")
    .split(/\r?\n/)
    .map((line) => line.trim().split(/\s+/)[0] || "")
    .filter((name) => name.endsWith(".scope"));
  for (const scope of scopes) {
    stopSystemdScope(scope);
  }
};

const shellQuote = (value) => `'${String(value).replace(/'/g, `'\"'\"'`)}'`;

const runChecked = (cmd, args, opts = {}) => {
  const result = spawnSync(cmd, args, { encoding: "utf8", ...opts });
  if (result.status === 0) return result;
  const stderr = String(result.stderr || "").trim();
  const stdout = String(result.stdout || "").trim();
  const detail = [stderr, stdout].filter(Boolean).join("\n");
  throw new Error(`${cmd} ${args.join(" ")} failed${detail ? `: ${detail}` : ""}`);
};

const resolveSshTarget = ({ host, user }) => {
  const normalizedHost = String(host || "").trim();
  if (!normalizedHost) return "";
  if (normalizedHost.includes("@")) return normalizedHost;
  const normalizedUser = String(user || "").trim();
  return normalizedUser ? `${normalizedUser}@${normalizedHost}` : normalizedHost;
};

const runSshCommand = ({ host, user, password, command }) => {
  const target = resolveSshTarget({ host, user });
  const sshArgs = [
    ...(REMOTE_SSH_KEY_PATH ? ["-F", "/dev/null", "-i", REMOTE_SSH_KEY_PATH, "-o", "IdentitiesOnly=yes"] : []),
    "-o", "StrictHostKeyChecking=no",
    "-o", "ConnectTimeout=15",
    "-o", "ServerAliveInterval=15",
    "-o", "ServerAliveCountMax=2",
    target,
    command,
  ];
  if (password && String(password).length > 0) {
    return runChecked("sshpass", ["-e", "ssh", ...sshArgs], {
      timeout: 45000,
      env: { ...process.env, SSHPASS: String(password) },
    });
  }
  return runChecked("ssh", sshArgs, { timeout: 45000 });
};

const runScpCommand = ({ host, user, password, localPath, remotePath }) => {
  const target = resolveSshTarget({ host, user });
  const scpArgs = [
    ...(REMOTE_SSH_KEY_PATH ? ["-F", "/dev/null", "-i", REMOTE_SSH_KEY_PATH, "-o", "IdentitiesOnly=yes"] : []),
    "-o", "StrictHostKeyChecking=no",
    "-o", "ConnectTimeout=15",
    "-o", "ServerAliveInterval=15",
    "-o", "ServerAliveCountMax=2",
    localPath,
    `${target}:${remotePath}`,
  ];
  if (password && String(password).length > 0) {
    return runChecked("sshpass", ["-e", "scp", ...scpArgs], {
      timeout: 180000,
      env: { ...process.env, SSHPASS: String(password) },
    });
  }
  return runChecked("scp", scpArgs, { timeout: 180000 });
};

const remoteDirname = (remotePath) => {
  const p = String(remotePath || "").trim();
  if (!p) return "/tmp";
  const idx = p.lastIndexOf("/");
  if (idx <= 0) return idx === 0 ? "/" : ".";
  return p.slice(0, idx);
};

const provisionRemoteCtxBinary = ({ host, user, password, remotePath }) => {
  if (!host || !remotePath) return;
  if (!fs.existsSync(CTX_BIN)) {
    throw new Error(`ctx binary not found at ${CTX_BIN} (required for remote provisioning)`);
  }
  try {
    runSshCommand({
      host,
      user,
      password,
      command: `test -x ${shellQuote(remotePath)}`,
    });
    return;
  } catch {
    // Fall through to copy if the binary is missing or not executable.
  }
  const parentDir = remoteDirname(remotePath);
  runSshCommand({
    host,
    user,
    password,
    command: `mkdir -p ${shellQuote(parentDir)}`,
  });
  runScpCommand({
    host,
    user,
    password,
    localPath: CTX_BIN,
    remotePath,
  });
  runSshCommand({
    host,
    user,
    password,
    command: `chmod 755 ${shellQuote(remotePath)}`,
  });
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

const desktopOs = () => {
  if (process.platform === "darwin") return "macos";
  if (process.platform === "win32") return "windows";
  return "linux";
};

const desktopArch = () => {
  if (process.arch === "arm64") return "aarch64";
  if (process.arch === "x64") return "x86_64";
  return process.arch;
};

const ensureBundledContainerAssets = () => {
  const manifestPath = path.join(BUNDLES_DIR, "manifest.json");
  if (!fs.existsSync(manifestPath)) {
    throw new Error(
      `bundled manifest missing at ${manifestPath}; run pnpm -C core desktop:prep:release`,
    );
  }
  const manifest = readJson(manifestPath);
  const runtimes = Array.isArray(manifest?.runtimes) ? manifest.runtimes : [];
  const images = Array.isArray(manifest?.images)
    ? manifest.images
    : Array.isArray(manifest?.harness_images)
      ? manifest.harness_images
      : [];

  const hostOs = desktopOs();
  const hostArch = desktopArch();
  const podmanRuntime = runtimes.find((entry) =>
    entry
    && entry.id === "podman"
    && entry.os === hostOs
    && entry.arch === hostArch
    && typeof entry.root === "string"
    && entry.root.trim().length > 0
    && typeof entry.bin === "string"
    && entry.bin.trim().length > 0
  );
  if (podmanRuntime) {
    const podmanBinPath = path.join(BUNDLES_DIR, podmanRuntime.root, podmanRuntime.bin);
    if (!fs.existsSync(podmanBinPath)) {
      throw new Error(
        `bundled podman binary missing at ${podmanBinPath}; run pnpm -C core desktop:prep:release`,
      );
    }
  } else {
    // Canonical path for thin bundles: daemon downloads managed podman runtime on demand.
    console.error(
      `[wdio] bundled podman runtime metadata missing for ${hostOs}/${hostArch}; relying on managed podman runtime download`,
    );
  }

  const harnessImage = images.find((entry) =>
    entry
    && entry.id === "ctx-harness"
    && entry.os === "linux"
    && entry.arch === hostArch
    && typeof entry.tar === "string"
    && entry.tar.trim().length > 0
  );
  if (harnessImage) {
    const harnessImageTar = path.join(BUNDLES_DIR, harnessImage.tar);
    if (!fs.existsSync(harnessImageTar)) {
      throw new Error(
        `bundled harness image tar missing at ${harnessImageTar}; run pnpm -C core desktop:prep:release`,
      );
    }
  } else {
    // Minimal bundle mode allows runtime pull for the default harness image.
    console.error(
      `[wdio] bundled harness image metadata missing for linux/${hostArch}; relying on runtime image pull`,
    );
  }
};

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
  if (SKIP_APP_BUILD) return;
  const appPathLooksLikeBundle = process.platform === "darwin" && APP_PATH.endsWith(".app");
  const darwinBundles = String(process.env.CTX_AUTOMATION_TAURI_BUNDLES || "app").trim() || "app";
  const buildArgs = appPathLooksLikeBundle
    ? ["tauri", "build", "--debug", "--bundles", darwinBundles, "--", "--features", "automation"]
    : ["tauri", "build", "--debug", "--no-bundle", "--", "--features", "automation"];
  const result = spawnSync(
    "pnpm",
    // On macOS, default APP_PATH is a .app bundle; build that by default.
    // On Linux/Windows, keep --no-bundle for faster automation iteration.
    buildArgs,
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
  // Run sequentially (Tauri + daemon + remote resources are shared global resources).
  specs: [path.resolve(__dirname, "specs/**/*.spec.cjs")],
  mochaOpts: {
    timeout: MOCHA_TIMEOUT_MS,
  },
  logLevel: WDIO_LOG_LEVEL,
  maxInstances: 1,
  capabilities: [
    {
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
    // Debug breadcrumb for remote-start behavior in automation logs.
    console.error(
      `[wdio] CTX_AUTOMATION_SSH_NO_START_REMOTE=${String(process.env.CTX_AUTOMATION_SSH_NO_START_REMOTE || "<unset>")} SSH_NO_START_REMOTE=${String(SSH_NO_START_REMOTE)}`,
    );
    const isDarwin = process.platform === "darwin";
    if (isDarwin && !process.env.CN_API_KEY) {
      throw new Error(
        "CN_API_KEY is required for CrabNebula WebDriver on macOS. " +
          "Load it from Infisical in core/ (core/.infisical.json), or run `pnpm -C core verify:desktop-smoke` which loads Infisical by default.",
      );
    }
    // Ensure we don't hit the single-instance path (which can forward to a stale app instance
    // without the automation plugin enabled).
    killExistingAppProcesses();
    killStaleAutomationHelpers();
    stopStaleSystemdScope();

    // Container-mode provider smoke needs a fully-bundled release-style resource set
    // (Linux provider binaries + harness image tars). Keep the app build in debug mode
    // for the automation plugin, but sync release resources.
    if (!SKIP_PREP_RELEASE) {
      const prepRelease = spawnSync("pnpm", ["-C", CORE_ROOT, "desktop:prep:release"], {
        stdio: "inherit",
        cwd: ROOT,
        shell: true,
      });
      if (prepRelease.status !== 0) {
        throw new Error("pnpm -C core desktop:prep:release failed");
      }
    }
    if (!process.env.CTX_BUNDLE_DIR) {
      process.env.CTX_BUNDLE_DIR = BUNDLES_DIR;
    }
    if (RUNS_CONTAINER_SCENARIOS) {
      ensureBundledContainerAssets();
    }

    // Launch the app directly into the wizard route to reduce test flakiness.
    process.env.CTX_DESKTOP_START_PATH = "/workspace-setup";
    process.env.CTX_SEED_CODEX_AUTH_FROM_HOST = process.env.CTX_SEED_CODEX_AUTH_FROM_HOST || "1";
    // Safety default: don't start/restart remote daemons unless explicitly enabled.
    if (SSH_NO_START_REMOTE) {
      process.env.CTX_DESKTOP_SSH_NO_START_REMOTE = "1";
      process.env.CTX_DESKTOP_SSH_START_REMOTE = "0";
    } else {
      process.env.CTX_DESKTOP_SSH_NO_START_REMOTE = "0";
      process.env.CTX_DESKTOP_SSH_START_REMOTE = "1";
    }
    console.error(
      `[wdio] CTX_DESKTOP_SSH_NO_START_REMOTE=${String(process.env.CTX_DESKTOP_SSH_NO_START_REMOTE || "<unset>")} CTX_DESKTOP_SSH_START_REMOTE=${String(process.env.CTX_DESKTOP_SSH_START_REMOTE || "<unset>")}`,
    );
    if (REMOTE_CTX_BIN) {
      if (!SSH_NO_START_REMOTE && !SKIP_REMOTE_CTX_PROVISION) {
        const targets = [];
        if (process.env.CTX_AUTOMATION_REMOTE_HOST) {
          targets.push({
            host: process.env.CTX_AUTOMATION_REMOTE_HOST,
            user: process.env.CTX_AUTOMATION_REMOTE_USER || "devboxadmin",
            password: process.env.CTX_AUTOMATION_REMOTE_PASSWORD || "",
          });
        }
        if (process.env.CTX_AUTOMATION_REMOTE_CONTAINER_HOST) {
          targets.push({
            host: process.env.CTX_AUTOMATION_REMOTE_CONTAINER_HOST,
            user: process.env.CTX_AUTOMATION_REMOTE_CONTAINER_USER || process.env.CTX_AUTOMATION_REMOTE_USER || "devboxadmin",
            password: process.env.CTX_AUTOMATION_REMOTE_CONTAINER_PASSWORD || process.env.CTX_AUTOMATION_REMOTE_PASSWORD || "",
          });
        }
        const seen = new Set();
        for (const t of targets) {
          const key = `${t.user || ""}@${t.host}:${REMOTE_CTX_BIN}`;
          if (seen.has(key)) continue;
          seen.add(key);
          provisionRemoteCtxBinary({
            host: t.host,
            user: t.user,
            password: t.password,
            remotePath: REMOTE_CTX_BIN,
          });
        }
      }
    }
    if (USE_EXTERNAL_DAEMON) {
      delete process.env.CTX_DESKTOP_DAEMON_DATA_DIR;
      await startExternalDaemon();
    } else {
      // Ensure we validate the real launcher path: the app must spawn/connect its own daemon.
      delete process.env.CTX_DESKTOP_DAEMON_URL;
      delete process.env.CTX_DESKTOP_DAEMON_TOKEN;
      internalDaemonDataDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-e2e-app-daemon-"));
      process.env.CTX_DESKTOP_DAEMON_DATA_DIR = internalDaemonDataDir;
    }

    buildAppIfMissing();

    if (isDarwin) {
      backendProcess = spawn("pnpm", ["exec", "test-runner-backend"], {
        stdio: "inherit",
        cwd: ROOT,
        env: {
          ...process.env,
          TEST_RUNNER_BACKEND_PORT: String(TEST_BACKEND_PORT),
        },
      });
      await waitTestRunnerBackendReady();
    }

    const driverEnv = {
      ...process.env,
      TAURI_DRIVER_PORT: String(TAURI_DRIVER_PORT),
    };
    if (isDarwin) {
      // On macOS, tauri-driver talks to CrabNebula's local backend.
      driverEnv.REMOTE_WEBDRIVER_URL = `http://127.0.0.1:${TEST_BACKEND_PORT}`;
    } else {
      // On Linux/Windows, tauri-driver drives platform WebDriver locally.
      delete driverEnv.REMOTE_WEBDRIVER_URL;
    }
    const useXvfbForDriver = process.platform === "linux" && !process.env.DISPLAY;
    const driverCmd = useXvfbForDriver ? "xvfb-run" : "pnpm";
    const driverArgs = useXvfbForDriver
      ? ["-a", "pnpm", "exec", "tauri-driver"]
      : ["exec", "tauri-driver"];
    driverProcess = spawn(driverCmd, driverArgs, {
      stdio: "inherit",
      cwd: ROOT,
      env: driverEnv,
    });
    await waitTauriDriverReady();
  },
  onComplete: (exitCode) => {
    const runFailed = Number(exitCode || 0) !== 0;
    const preserveDaemonArtifacts = runFailed;
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
        if (!preserveDaemonArtifacts) {
          fs.rmSync(daemonDataDir, { recursive: true, force: true });
        } else {
          console.error(`[wdio] preserving daemonDataDir (test failure): ${daemonDataDir}`);
        }
      } catch {
        // ignore
      }
      if (!preserveDaemonArtifacts) {
        daemonDataDir = null;
        daemonLogPath = null;
      }
    }
    if (internalDaemonDataDir) {
      try {
        if (!preserveDaemonArtifacts) {
          fs.rmSync(internalDaemonDataDir, { recursive: true, force: true });
        } else {
          console.error(
            `[wdio] preserving internalDaemonDataDir (test failure): ${internalDaemonDataDir}`,
          );
        }
      } catch {
        // ignore
      }
      if (!preserveDaemonArtifacts) {
        internalDaemonDataDir = null;
      }
    }
  },
};
