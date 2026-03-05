const path = require("path");
const fs = require("fs");
const crypto = require("crypto");
const { spawnSync, spawn } = require("child_process");
const os = require("os");

const { waitTestRunnerBackendReady } = require("@crabnebula/test-runner-backend");
const { waitTauriDriverReady } = require("@crabnebula/tauri-driver");
const TEST_RUNNER_BACKEND_CLI = require.resolve("@crabnebula/test-runner-backend/cli.js");
const TAURI_DRIVER_CLI = require.resolve("@crabnebula/tauri-driver/cli.js");

const ROOT = path.resolve(__dirname, "..");
const CORE_ROOT = path.resolve(ROOT, "..", "..");
const defaultAppPath = (() => {
  if (process.platform === "darwin") {
    return path.resolve(ROOT, "src-tauri/target/debug/bundle/macos/ctx.app/Contents/MacOS/ctx");
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

const parsePort = (raw, fallback) => {
  const n = Number.parseInt(String(raw ?? ""), 10);
  if (!Number.isFinite(n) || n <= 0 || n > 65535) return fallback;
  return n;
};

const pickUnusedPortSync = (fallback) => {
  const script = [
    "const net = require('node:net');",
    "const s = net.createServer();",
    "s.on('error', () => process.exit(2));",
    "s.listen(0, '127.0.0.1', () => {",
    "  const addr = s.address();",
    "  const p = addr && typeof addr === 'object' ? addr.port : 0;",
    "  s.close(() => {",
    "    if (!p) process.exit(3);",
    "    process.stdout.write(String(p));",
    "  });",
    "});",
  ].join("\n");
  const out = spawnSync(process.execPath, ["-e", script], { encoding: "utf8" });
  if (out.status !== 0) return fallback;
  return parsePort(String(out.stdout || "").trim(), fallback);
};

const DEFAULT_DRIVER_PORT = process.platform === "darwin"
  ? pickUnusedPortSync(4444)
  : 4444;
const TAURI_DRIVER_PORT = parsePort(process.env.TAURI_DRIVER_PORT, DEFAULT_DRIVER_PORT);
const TEST_BACKEND_PORT = parsePort(process.env.TAURI_TEST_BACKEND_PORT, 3000);
const MACOS_CN_BACKEND_PORT = parsePort(process.env.CTX_AUTOMATION_CN_BACKEND_PORT, 3000);
if (!String(process.env.TAURI_DRIVER_PORT || "").trim()) {
  // WDIO forks workers that reload this config; pin the chosen dynamic port for all children.
  process.env.TAURI_DRIVER_PORT = String(TAURI_DRIVER_PORT);
}

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
const REMOTE_SSH_CONFIG_PATH = String(
  process.env.CTX_DESKTOP_SSH_CONFIG_PATH || process.env.CTX_AUTOMATION_REMOTE_FIXTURE_SSH_CONFIG || "",
).trim();
const SKIP_REMOTE_CTX_PROVISION = ["1", "true", "yes"].includes(
  String(process.env.CTX_AUTOMATION_SKIP_REMOTE_CTX_PROVISION || "0").trim().toLowerCase(),
);
const WDIO_LOG_LEVEL = String(process.env.CTX_AUTOMATION_WDIO_LOG_LEVEL || "info").trim() || "info";
const INTERNAL_DAEMON_DATA_DIR_OVERRIDE = String(
  process.env.CTX_AUTOMATION_INTERNAL_DAEMON_DATA_DIR || "",
).trim();
const PRESERVE_INTERNAL_DAEMON_DATA_DIR = ["1", "true", "yes"].includes(
  String(process.env.CTX_AUTOMATION_PRESERVE_INTERNAL_DAEMON_DATA_DIR || "0").trim().toLowerCase(),
);
const parsePositiveInt = (raw, fallback) => {
  const n = Number.parseInt(String(raw ?? ""), 10);
  if (!Number.isFinite(n) || n <= 0) return fallback;
  return n;
};
const resolveMochaTimeoutMs = () => parsePositiveInt(
  process.env.CTX_AUTOMATION_MOCHA_TIMEOUT_MS
    || process.env.CTX_AUTOMATION_CASE_TIMEOUT_MS
    || "1200000",
  1200000,
);
const MOCHA_TIMEOUT_MS = resolveMochaTimeoutMs();
const CN_PORT_WAIT_MS = parsePositiveInt(process.env.CTX_AUTOMATION_CN_PORT_WAIT_MS || "120000", 120000);
const CN_BACKEND_LOCK_TIMEOUT_MS = parsePositiveInt(
  process.env.CTX_AUTOMATION_CN_BACKEND_LOCK_TIMEOUT_MS || "30000",
  30000,
);
const CN_BACKEND_LOCK_POLL_MS = parsePositiveInt(
  process.env.CTX_AUTOMATION_CN_BACKEND_LOCK_POLL_MS || "125",
  125,
);
const CN_BACKEND_LOCK_STALE_MS = parsePositiveInt(
  process.env.CTX_AUTOMATION_CN_BACKEND_LOCK_STALE_MS || "120000",
  120000,
);
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
const ALLOW_CN_PORT_REUSE = ["1", "true", "yes"].includes(
  String(process.env.CTX_AUTOMATION_CN_ALLOW_PORT_REUSE || "0").trim().toLowerCase(),
);
const SHARED_CN_BACKEND = ["0", "false", "no"].includes(
  String(process.env.CTX_AUTOMATION_CN_SHARED_BACKEND || "1").trim().toLowerCase(),
) ? false : process.platform === "darwin";
const STOP_SHARED_CN_BACKEND_WHEN_IDLE = ["1", "true", "yes"].includes(
  String(process.env.CTX_AUTOMATION_CN_STOP_SHARED_BACKEND_WHEN_IDLE || "0").trim().toLowerCase(),
);
const defaultCnBackendStateDir = (() => {
  if (process.platform === "darwin") {
    return path.join(os.homedir(), "Library", "Caches", "ctx-cn-backend");
  }
  if (process.platform === "win32") {
    const base = String(process.env.LOCALAPPDATA || os.tmpdir()).trim();
    return path.join(base, "ctx-cn-backend");
  }
  return path.join(os.homedir(), ".cache", "ctx-cn-backend");
})();
const CN_BACKEND_STATE_DIR = String(
  process.env.CTX_AUTOMATION_CN_BACKEND_STATE_DIR || defaultCnBackendStateDir,
).trim();
const CN_BACKEND_LOCK_FILE = path.join(CN_BACKEND_STATE_DIR, "backend.lock");
const CN_BACKEND_STATE_FILE = path.join(CN_BACKEND_STATE_DIR, "backend.json");
const CN_BACKEND_LEASES_DIR = path.join(CN_BACKEND_STATE_DIR, "leases");
const SHARED_CN_BACKEND_ENV_PREFIXES = [
  "CTX_DESKTOP_",
  "CTX_AUTOMATION_REMOTE_",
];
const SHARED_CN_BACKEND_ENV_KEYS = new Set([
  "CTX_BUNDLE_DIR",
  "CTX_SEED_CODEX_AUTH_FROM_HOST",
]);
const ALLOW_STALE_HELPER_SWEEP = ["1", "true", "yes"].includes(
  String(process.env.CTX_AUTOMATION_ALLOW_STALE_HELPER_SWEEP || "0").trim().toLowerCase(),
);
const ALLOW_PREP_APP_PROCESS_SWEEP = ["1", "true", "yes"].includes(
  String(process.env.CTX_AUTOMATION_ALLOW_PREP_APP_PROCESS_SWEEP || "0").trim().toLowerCase(),
);

let daemonProcess = null;
let daemonDataDir = null;
let daemonPort = null;
let daemonLogPath = null;
let internalDaemonDataDir = null;
let activeTestBackendPort = TEST_BACKEND_PORT;
let activeTauriDriverPort = TAURI_DRIVER_PORT;
let cnBackendLeaseId = null;
let cnBackendOwnedBySharedManager = false;

const resolveMacAppBundleDir = (appPath) => {
  const normalized = path.resolve(appPath);
  if (normalized.endsWith(".app")) return normalized;
  const marker = `${path.sep}.app${path.sep}`;
  const idx = normalized.indexOf(marker);
  if (idx === -1) return null;
  return normalized.slice(0, idx + marker.length - 1);
};

const resolveAppExecutablePath = (appPath) => {
  if (!fs.existsSync(appPath)) return appPath;
  const stat = fs.statSync(appPath);
  if (stat.isDirectory()) {
    return path.resolve(appPath, "Contents", "MacOS", "ctx");
  }
  return appPath;
};

const resolveAppResourcesBinPrefix = (appPath) => {
  const bundleDir = process.platform === "darwin" ? resolveMacAppBundleDir(appPath) : null;
  if (bundleDir) {
    return path.resolve(bundleDir, "Contents", "Resources", "bin");
  }
  return path.resolve(path.dirname(appPath), "bin");
};

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
  const appBin = resolveAppExecutablePath(APP_PATH);
  const appResBinPrefix = resolveAppResourcesBinPrefix(APP_PATH);
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

const ensureAppExecutable = () => {
  if (!fs.existsSync(APP_PATH)) {
    throw new Error(`desktop app path not found: ${APP_PATH}`);
  }
  if (process.platform !== "darwin") return;
  const appBin = resolveAppExecutablePath(APP_PATH);
  if (!fs.existsSync(appBin)) {
    throw new Error(`desktop app binary missing or not built: ${appBin}`);
  }
  const mode = fs.statSync(appBin).mode & 0o777;
  if ((mode & 0o111) === 0) {
    throw new Error(
      `desktop app binary is not executable (mode ${mode.toString(8)}): ${appBin}. ` +
        "Ensure release artifact restore preserves or reapplies +x before updater smoke.",
    );
  }
};

const killStaleAutomationHelpers = () => {
  // WebKit webdriver helpers can survive backend shutdown and keep pipes open.
  killProcesses((_pid, cmd) =>
    /\bWebKitWebDriver\b/.test(cmd) ||
    /\bwkwebdriver\b/.test(cmd),
  );
};

const ensureDesktopDevBinDir = () => {
  const configured = String(process.env.CTX_DESKTOP_DEV_BIN_DIR || "").trim();
  if (configured) return;
  const defaultBinName = process.platform === "win32" ? "ctx.exe" : "ctx";
  const candidateDir = path.resolve(path.dirname(CTX_BIN));
  const candidateBin = path.join(candidateDir, defaultBinName);
  if (!fs.existsSync(candidateBin)) return;
  process.env.CTX_DESKTOP_DEV_BIN_DIR = candidateDir;
  console.error(`[wdio] CTX_DESKTOP_DEV_BIN_DIR=${candidateDir}`);
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
  const sshConfigArgs = REMOTE_SSH_CONFIG_PATH
    ? ["-F", REMOTE_SSH_CONFIG_PATH]
    : [];
  const sshIdentityArgs = REMOTE_SSH_KEY_PATH
    ? ["-i", REMOTE_SSH_KEY_PATH, "-o", "IdentitiesOnly=yes"]
    : [];
  const sshDefaultConfigArgs = (!REMOTE_SSH_CONFIG_PATH && REMOTE_SSH_KEY_PATH)
    ? ["-F", "/dev/null"]
    : [];
  const sshArgs = [
    ...sshConfigArgs,
    ...sshDefaultConfigArgs,
    ...sshIdentityArgs,
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
  const scpConfigArgs = REMOTE_SSH_CONFIG_PATH
    ? ["-F", REMOTE_SSH_CONFIG_PATH]
    : [];
  const scpIdentityArgs = REMOTE_SSH_KEY_PATH
    ? ["-i", REMOTE_SSH_KEY_PATH, "-o", "IdentitiesOnly=yes"]
    : [];
  const scpDefaultConfigArgs = (!REMOTE_SSH_CONFIG_PATH && REMOTE_SSH_KEY_PATH)
    ? ["-F", "/dev/null"]
    : [];
  const scpArgs = [
    ...scpConfigArgs,
    ...scpDefaultConfigArgs,
    ...scpIdentityArgs,
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
let backendLogFd = null;
let driverLogFd = null;
let backendCliAliasDir = null;
let driverCliAliasDir = null;

const attachProcessDiagnostics = (name, proc) => {
  if (!proc) return;
  proc.on("error", (err) => {
    console.error(`[wdio] ${name} error: ${String(err)}`);
  });
  proc.on("exit", (code, signal) => {
    console.error(`[wdio] ${name} exited (code=${code ?? "null"}, signal=${signal ?? "null"})`);
  });
};

const waitForProcessReady = async ({ proc, name, readyPromise, detail = "" }) => {
  if (!proc) {
    throw new Error(`${name} process missing before readiness check`);
  }
  let onExit = null;
  const exitedBeforeReady = new Promise((_, reject) => {
    onExit = (code, signal) => {
      const suffix = detail ? ` ${detail}` : "";
      reject(new Error(
        `${name} exited before ready (code=${code ?? "null"}, signal=${signal ?? "null"}).${suffix}`,
      ));
    };
    proc.once("exit", onExit);
  });
  try {
    await Promise.race([readyPromise, exitedBeforeReady]);
  } finally {
    if (onExit) {
      proc.off("exit", onExit);
    }
  }
};

const isTcpPortOpen = (host, port, timeoutMs = 250) =>
  new Promise((resolve) => {
    const net = require("net");
    const socket = net.createConnection({ host, port });
    let settled = false;
    const finish = (ok) => {
      if (settled) return;
      settled = true;
      try {
        socket.destroy();
      } catch {
        // ignore
      }
      resolve(ok);
    };
    socket.setTimeout(timeoutMs);
    socket.once("connect", () => finish(true));
    socket.once("timeout", () => finish(false));
    socket.once("error", () => finish(false));
  });

const waitForTcpPortClosed = async (host, port, timeoutMs, pollMs = 500) => {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    const isOpen = await isTcpPortOpen(host, port);
    if (!isOpen) return true;
    await new Promise((resolve) => setTimeout(resolve, pollMs));
  }
  return !(await isTcpPortOpen(host, port));
};

const createCliAlias = (targetCliPath, aliasName) => {
  const aliasDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-cn-cli-"));
  const cliPath = path.join(aliasDir, aliasName);
  fs.symlinkSync(targetCliPath, cliPath);
  return { aliasDir, cliPath };
};

const canonicalPath = (p) => {
  const raw = String(p || "").trim();
  if (!raw) return raw;
  try {
    if (typeof fs.realpathSync.native === "function") {
      return fs.realpathSync.native(raw);
    }
    return fs.realpathSync(raw);
  } catch {
    return raw;
  }
};

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const parsePid = (raw) => {
  const n = Number.parseInt(String(raw ?? ""), 10);
  if (!Number.isFinite(n) || n <= 0) return null;
  return n;
};

const isProcessAlive = (pid) => {
  const n = parsePid(pid);
  if (!n) return false;
  try {
    process.kill(n, 0);
    return true;
  } catch (err) {
    return Boolean(err && err.code === "EPERM");
  }
};

const readJsonFile = (filePath) => {
  try {
    if (!fs.existsSync(filePath)) return null;
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch {
    return null;
  }
};

const writeJsonFileAtomic = (filePath, value) => {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  const tmpPath = `${filePath}.${process.pid}.${Date.now()}.tmp`;
  fs.writeFileSync(tmpPath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
  fs.renameSync(tmpPath, filePath);
};

const removeFileIfExists = (filePath) => {
  try {
    fs.rmSync(filePath, { force: true });
  } catch {
    // ignore
  }
};

const withFileLock = async (lockPath, timeoutMs, fn) => {
  fs.mkdirSync(path.dirname(lockPath), { recursive: true });
  const deadline = Date.now() + timeoutMs;
  while (true) {
    let lockFd = null;
    try {
      lockFd = fs.openSync(
        lockPath,
        fs.constants.O_CREAT | fs.constants.O_EXCL | fs.constants.O_WRONLY,
        0o600,
      );
      fs.writeFileSync(
        lockFd,
        `${JSON.stringify({ pid: process.pid, acquiredAt: new Date().toISOString() })}\n`,
        "utf8",
      );
      try {
        return await fn();
      } finally {
        try {
          fs.closeSync(lockFd);
        } catch {
          // ignore
        }
        removeFileIfExists(lockPath);
      }
    } catch (err) {
      if (lockFd !== null) {
        try {
          fs.closeSync(lockFd);
        } catch {
          // ignore
        }
      }
      if (!err || err.code !== "EEXIST") throw err;

      let staleLock = false;
      try {
        const lockStat = fs.statSync(lockPath);
        staleLock = (Date.now() - Number(lockStat.mtimeMs || 0)) > CN_BACKEND_LOCK_STALE_MS;
      } catch {
        staleLock = true;
      }
      if (!staleLock) {
        const lockInfo = readJsonFile(lockPath);
        if (lockInfo && !isProcessAlive(lockInfo.pid)) {
          staleLock = true;
        }
      }
      if (staleLock) {
        removeFileIfExists(lockPath);
        continue;
      }
      if (Date.now() >= deadline) {
        throw new Error(
          `[wdio] timed out acquiring lock ${lockPath} after ${timeoutMs}ms`,
        );
      }
      await sleep(CN_BACKEND_LOCK_POLL_MS);
    }
  }
};

const withCnBackendLock = async (fn) =>
  withFileLock(CN_BACKEND_LOCK_FILE, CN_BACKEND_LOCK_TIMEOUT_MS, fn);

const cnLeasePath = (leaseId) => path.join(CN_BACKEND_LEASES_DIR, `${leaseId}.json`);

const listCnLeasePaths = () => {
  if (!fs.existsSync(CN_BACKEND_LEASES_DIR)) return [];
  return fs
    .readdirSync(CN_BACKEND_LEASES_DIR)
    .filter((name) => name.endsWith(".json"))
    .map((name) => path.join(CN_BACKEND_LEASES_DIR, name));
};

const cleanupStaleCnLeases = () => {
  fs.mkdirSync(CN_BACKEND_LEASES_DIR, { recursive: true });
  const active = [];
  for (const leasePath of listCnLeasePaths()) {
    const lease = readJsonFile(leasePath);
    if (!lease) {
      removeFileIfExists(leasePath);
      continue;
    }
    const leasePid = parsePid(lease.pid);
    const leasePidAlive = leasePid ? isProcessAlive(leasePid) : false;
    const leaseId = String(lease.leaseId || path.basename(leasePath, ".json")).trim();
    if (!leasePid || !leaseId || !leasePidAlive) {
      removeFileIfExists(leasePath);
      continue;
    }
    active.push({ leaseId, pid: leasePid });
  }
  return active;
};

const readCnBackendState = () => {
  const state = readJsonFile(CN_BACKEND_STATE_FILE);
  return state && typeof state === "object" ? state : null;
};

const writeCnBackendState = (state) => {
  writeJsonFileAtomic(CN_BACKEND_STATE_FILE, state);
};

const createCnLeaseId = () =>
  `${process.pid}-${Date.now().toString(36)}-${Math.random().toString(16).slice(2, 10)}`;

const openBackendStdio = ({ detached = false } = {}) => {
  const backendLogPath = String(process.env.CTX_AUTOMATION_CN_BACKEND_LOG || "").trim();
  if (!backendLogPath) {
    return detached ? ["ignore", "ignore", "ignore"] : "inherit";
  }
  fs.mkdirSync(path.dirname(backendLogPath), { recursive: true });
  backendLogFd = fs.openSync(backendLogPath, "a");
  return ["ignore", backendLogFd, backendLogFd];
};

const spawnCnBackendProcess = (host, port, { detached = false } = {}) => {
  const backendAlias = createCliAlias(TEST_RUNNER_BACKEND_CLI, "ctx-cnb-cli");
  backendCliAliasDir = backendAlias.aliasDir;
  const stdio = openBackendStdio({ detached });
  const proc = spawn(
    process.execPath,
    [backendAlias.cliPath, "--host", host, "--port", String(port)],
    {
      stdio,
      cwd: ROOT,
      detached,
      env: {
        ...process.env,
        TEST_RUNNER_BACKEND_PORT: String(port),
      },
    },
  );
  attachProcessDiagnostics("test-runner-backend", proc);
  return proc;
};

const collectSharedCnBackendLaunchEnv = () => {
  const payload = {};
  for (const key of Object.keys(process.env).sort()) {
    const value = String(process.env[key] || "");
    if (!value) continue;
    if (
      SHARED_CN_BACKEND_ENV_KEYS.has(key)
      || SHARED_CN_BACKEND_ENV_PREFIXES.some((prefix) => key.startsWith(prefix))
    ) {
      payload[key] = value;
    }
  }
  return payload;
};

const computeSharedCnBackendLaunchEnvSignature = (payload) => {
  if (!payload || Object.keys(payload).length === 0) return null;
  return crypto
    .createHash("sha256")
    .update(JSON.stringify(payload))
    .digest("hex");
};

const stopSharedCnBackendProcess = async (pid) => {
  const n = parsePid(pid);
  if (!n || !isProcessAlive(n)) return;
  try {
    process.kill(n, "SIGTERM");
  } catch {
    // ignore
  }
  await sleep(500);
  if (!isProcessAlive(n)) return;
  try {
    process.kill(n, "SIGKILL");
  } catch {
    // ignore
  }
};

const acquireSharedCnBackendLease = async (host, port) => {
  const leaseId = createCnLeaseId();
  const leaseFile = cnLeasePath(leaseId);
  let startedByThisRun = false;
  let startedPid = null;
  const launchEnvPayload = collectSharedCnBackendLaunchEnv();
  const launchEnvSignature = computeSharedCnBackendLaunchEnvSignature(launchEnvPayload);

  await withCnBackendLock(async () => {
    fs.mkdirSync(CN_BACKEND_LEASES_DIR, { recursive: true });
    const activeLeases = cleanupStaleCnLeases();
    let existingState = readCnBackendState() || {};
    let existingPid = parsePid(existingState.pid);
    let existingPidAlive = existingPid ? isProcessAlive(existingPid) : false;
    let portOpen = await isTcpPortOpen(host, port);
    writeJsonFileAtomic(leaseFile, {
      leaseId,
      pid: process.pid,
      host,
      port,
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
    });

    const existingSignatureTrusted = existingState.launchEnvSignatureSource === "spawn";
    const launchEnvNeedsRestart = Boolean(launchEnvSignature) && (
      !existingSignatureTrusted
      || (
        String(existingState.launchEnvSignature || "") !== ""
        && existingState.launchEnvSignature !== launchEnvSignature
      )
    );
    if (launchEnvNeedsRestart) {
      if (activeLeases.length > 0) {
        throw new Error(
          `[wdio] shared CN backend launch environment changed while ${activeLeases.length} other lease(s) are active; cannot safely reuse backend`,
        );
      }
      if (backendProcess && cnBackendOwnedBySharedManager) {
        try {
          backendProcess.kill();
        } catch {
          // ignore
        }
        backendProcess = null;
      } else if (existingPidAlive) {
        await stopSharedCnBackendProcess(existingPid);
      }
      removeFileIfExists(CN_BACKEND_STATE_FILE);
      existingState = {};
      existingPid = null;
      existingPidAlive = false;
      portOpen = false;
      console.error(`[wdio] restarting shared test-runner-backend for updated launch environment`);
    }

    if (portOpen || existingPidAlive) {
      writeCnBackendState({
        ...existingState,
        host,
        port,
        pid: existingPidAlive ? existingPid : existingState.pid || null,
        updatedAt: new Date().toISOString(),
      });
      return;
    }

    backendProcess = spawnCnBackendProcess(host, port, { detached: true });
    cnBackendOwnedBySharedManager = true;
    startedByThisRun = true;
    startedPid = parsePid(backendProcess.pid);
    writeCnBackendState({
      host,
      port,
      pid: startedPid,
      startedByPid: process.pid,
      startedByLeaseId: leaseId,
      launchEnvSignature,
      launchEnvSignatureSource: "spawn",
      startedAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
    });
    console.error(
      `[wdio] starting shared test-runner-backend on ${host}:${port} (activeLeases=${activeLeases.length})`,
    );
  });

  try {
    if (startedByThisRun) {
      await waitForProcessReady({
        proc: backendProcess,
        name: "test-runner-backend",
        readyPromise: waitTestRunnerBackendReady(host, port),
        detail: `Shared backend startup for ${host}:${port} failed before readiness.`,
      });
      console.error(`[wdio] shared test-runner-backend ready at ${host}:${port}`);
    } else {
      await waitTestRunnerBackendReady(host, port);
      console.error(`[wdio] reusing shared test-runner-backend at ${host}:${port}`);
    }
    cnBackendLeaseId = leaseId;
    return;
  } catch (err) {
    const backendReady = await isTcpPortOpen(host, port);
    if (backendReady) {
      await waitTestRunnerBackendReady(host, port);
      console.error(`[wdio] shared backend race recovered at ${host}:${port}`);
      cnBackendLeaseId = leaseId;
      return;
    }
    await withCnBackendLock(async () => {
      removeFileIfExists(leaseFile);
      const state = readCnBackendState();
      const statePid = parsePid(state && state.pid);
      if (startedByThisRun && startedPid && statePid && statePid === startedPid) {
        removeFileIfExists(CN_BACKEND_STATE_FILE);
      }
    });
    throw err;
  }
};

const releaseSharedCnBackendLease = async (host, port) => {
  const leaseId = cnBackendLeaseId;
  if (!leaseId) return;
  cnBackendLeaseId = null;
  const leaseFile = cnLeasePath(leaseId);

  await withCnBackendLock(async () => {
    removeFileIfExists(leaseFile);
    const activeLeases = cleanupStaleCnLeases();
    if (activeLeases.length > 0) {
      console.error(
        `[wdio] released shared CN backend lease ${leaseId}; remainingLeases=${activeLeases.length}`,
      );
      return;
    }

    if (!STOP_SHARED_CN_BACKEND_WHEN_IDLE) {
      console.error(
        `[wdio] released shared CN backend lease ${leaseId}; backend remains running on ${host}:${port}`,
      );
      return;
    }

    const state = readCnBackendState();
    const statePid = parsePid(state && state.pid);
    if (backendProcess && cnBackendOwnedBySharedManager) {
      try {
        backendProcess.kill();
      } catch {
        // ignore
      }
      backendProcess = null;
    } else if (statePid && isProcessAlive(statePid)) {
      try {
        process.kill(statePid, "SIGTERM");
      } catch {
        // ignore
      }
      await sleep(500);
      if (isProcessAlive(statePid)) {
        try {
          process.kill(statePid, "SIGKILL");
        } catch {
          // ignore
        }
      }
    }
    removeFileIfExists(CN_BACKEND_STATE_FILE);
    console.error(`[wdio] stopped shared test-runner-backend after final lease release (${host}:${port})`);
  });
  if (backendProcess && cnBackendOwnedBySharedManager) {
    try {
      backendProcess.unref();
    } catch {
      // ignore
    }
    backendProcess = null;
  }
  cnBackendOwnedBySharedManager = false;
};

const cnSharedBackendTestHooks = {
  getPaths: () => ({
    stateDir: CN_BACKEND_STATE_DIR,
    lockFile: CN_BACKEND_LOCK_FILE,
    stateFile: CN_BACKEND_STATE_FILE,
    leasesDir: CN_BACKEND_LEASES_DIR,
  }),
  createLeaseId: createCnLeaseId,
  leasePath: cnLeasePath,
  withLock: withCnBackendLock,
  cleanupStaleLeases: cleanupStaleCnLeases,
  readState: readCnBackendState,
  writeState: writeCnBackendState,
  writeJsonFileAtomic,
  removeFileIfExists,
  releaseSharedCnBackendLease,
  setCurrentLeaseId: (leaseId) => {
    cnBackendLeaseId = leaseId;
  },
};

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
      timeouts: {
        script: 180000,
        pageLoad: 300000,
        implicit: 0,
      },
      "tauri:options": {
        application: APP_PATH,
      },
    },
  ],
  port: activeTauriDriverPort,
  path: "/",
  automationProtocol: "webdriver",
  beforeSession: () => {
    process.env.CTX_AUTOMATION_WORKSPACE_PATH = WORKSPACE_PATH;
  },
  onPrepare: async () => {
    // Debug breadcrumb for remote-start behavior in automation logs.
    const isDarwin = process.platform === "darwin";
    console.error(
      `[wdio] CTX_AUTOMATION_SSH_NO_START_REMOTE=${String(process.env.CTX_AUTOMATION_SSH_NO_START_REMOTE || "<unset>")} SSH_NO_START_REMOTE=${String(SSH_NO_START_REMOTE)}`,
    );
    console.error(`[wdio] app path=${APP_PATH}`);
    activeTauriDriverPort = TAURI_DRIVER_PORT;
    console.error(
      `[wdio] ports driver(requested)=${String(TAURI_DRIVER_PORT)} driver(effective)=${String(activeTauriDriverPort)} backend(requested)=${String(TEST_BACKEND_PORT)}`,
    );
    if (isDarwin && !process.env.CN_API_KEY) {
      throw new Error(
        "CN_API_KEY is required for CrabNebula WebDriver on macOS. " +
          "Load it from Infisical in core/ (core/.infisical.json), or run `pnpm -C core verify:desktop-smoke` which loads Infisical by default.",
      );
    }
    if (ALLOW_PREP_APP_PROCESS_SWEEP) {
      // Automation builds disable single-instance mode; only use global app sweeps when explicitly requested.
      killExistingAppProcesses();
    }
    if (ALLOW_STALE_HELPER_SWEEP && !(isDarwin && SHARED_CN_BACKEND)) {
      killStaleAutomationHelpers();
    }
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
    // Debug desktop binaries resolve local daemon executables from this directory.
    ensureDesktopDevBinDir();
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
      if (INTERNAL_DAEMON_DATA_DIR_OVERRIDE) {
        internalDaemonDataDir = path.resolve(INTERNAL_DAEMON_DATA_DIR_OVERRIDE);
        fs.mkdirSync(internalDaemonDataDir, { recursive: true });
        internalDaemonDataDir = canonicalPath(internalDaemonDataDir);
      } else {
        internalDaemonDataDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-e2e-app-daemon-"));
        internalDaemonDataDir = canonicalPath(internalDaemonDataDir);
      }
      process.env.CTX_DESKTOP_DAEMON_DATA_DIR = internalDaemonDataDir;
    }

    buildAppIfMissing();
    // Validate executable/bundle after optional build so clean machines can self-bootstrap.
    ensureAppExecutable();

    if (isDarwin) {
      activeTestBackendPort = MACOS_CN_BACKEND_PORT;
      if (TEST_BACKEND_PORT !== MACOS_CN_BACKEND_PORT) {
        console.error(
          `[wdio] macOS backend currently binds ${MACOS_CN_BACKEND_PORT}; ignoring requested TAURI_TEST_BACKEND_PORT=${TEST_BACKEND_PORT}`,
        );
      }
      const backendHost = "127.0.0.1";
      if (SHARED_CN_BACKEND) {
        await acquireSharedCnBackendLease(backendHost, activeTestBackendPort);
      } else {
        let backendAlreadyRunning = await isTcpPortOpen(backendHost, activeTestBackendPort);
        if (backendAlreadyRunning) {
          if (!ALLOW_CN_PORT_REUSE && CN_PORT_WAIT_MS > 0) {
            console.error(
              `[wdio] waiting for CrabNebula backend port ${activeTestBackendPort} to become free (timeout=${CN_PORT_WAIT_MS}ms)`,
            );
            await waitForTcpPortClosed(backendHost, activeTestBackendPort, CN_PORT_WAIT_MS);
            backendAlreadyRunning = await isTcpPortOpen(backendHost, activeTestBackendPort);
          }
          if (backendAlreadyRunning && !ALLOW_CN_PORT_REUSE) {
            throw new Error(
              `CrabNebula backend port ${activeTestBackendPort} is already in use at ${backendHost}. ` +
                "Refusing to reuse an existing backend by default to avoid cross-run contamination. " +
                "Ensure no other desktop automation run is active, or set CTX_AUTOMATION_CN_ALLOW_PORT_REUSE=1.",
            );
          }
          if (backendAlreadyRunning) {
            console.error(
              `[wdio] reusing existing test-runner-backend at ${backendHost}:${activeTestBackendPort}`,
            );
            await waitTestRunnerBackendReady(backendHost, activeTestBackendPort);
          }
        }
        if (!backendAlreadyRunning) {
          backendProcess = spawnCnBackendProcess(backendHost, activeTestBackendPort);
          await waitForProcessReady({
            proc: backendProcess,
            name: "test-runner-backend",
            readyPromise: waitTestRunnerBackendReady(backendHost, activeTestBackendPort),
            detail: `On macOS the backend may contend on port ${activeTestBackendPort}; ensure no other desktop automation run is active.`,
          });
        }
      }
    }

    const driverEnv = {
      ...process.env,
      TAURI_DRIVER_PORT: String(activeTauriDriverPort),
    };
    if (isDarwin) {
      // On macOS, tauri-driver talks to CrabNebula's local backend.
      driverEnv.REMOTE_WEBDRIVER_URL = `http://127.0.0.1:${activeTestBackendPort}`;
    } else {
      // On Linux/Windows, tauri-driver drives platform WebDriver locally.
      delete driverEnv.REMOTE_WEBDRIVER_URL;
    }
    let driverCmd = "";
    let driverArgs = [];
    if (isDarwin) {
      // Use a neutral CLI alias so shared-host kill sweeps targeting
      // "tauri-driver" command names do not terminate this run.
      const driverAlias = createCliAlias(TAURI_DRIVER_CLI, "ctx-tdrv-cli");
      driverCliAliasDir = driverAlias.aliasDir;
      driverCmd = process.execPath;
      driverArgs = [driverAlias.cliPath, "--port", String(activeTauriDriverPort)];
    } else {
      const useXvfbForDriver = process.platform === "linux" && !process.env.DISPLAY;
      driverCmd = useXvfbForDriver ? "xvfb-run" : "pnpm";
      driverArgs = useXvfbForDriver
        ? ["-a", "pnpm", "exec", "tauri-driver"]
        : ["exec", "tauri-driver"];
    }
    const driverLogPath = String(process.env.CTX_AUTOMATION_CN_DRIVER_LOG || "").trim();
    let driverStdio = "inherit";
    if (driverLogPath) {
      fs.mkdirSync(path.dirname(driverLogPath), { recursive: true });
      driverLogFd = fs.openSync(driverLogPath, "a");
      driverStdio = ["ignore", driverLogFd, driverLogFd];
    }
    const driverHost = "127.0.0.1";
    let driverAlreadyRunning = await isTcpPortOpen(driverHost, activeTauriDriverPort);
    if (driverAlreadyRunning) {
      if (!ALLOW_CN_PORT_REUSE && CN_PORT_WAIT_MS > 0) {
        console.error(
          `[wdio] waiting for tauri-driver port ${activeTauriDriverPort} to become free (timeout=${CN_PORT_WAIT_MS}ms)`,
        );
        await waitForTcpPortClosed(driverHost, activeTauriDriverPort, CN_PORT_WAIT_MS);
        driverAlreadyRunning = await isTcpPortOpen(driverHost, activeTauriDriverPort);
      }
      if (driverAlreadyRunning && !ALLOW_CN_PORT_REUSE) {
        throw new Error(
          `tauri-driver port ${activeTauriDriverPort} is already in use at ${driverHost}. ` +
            "Refusing to reuse an existing driver by default to avoid cross-run contamination. " +
            "Ensure no other desktop automation run is active, or set CTX_AUTOMATION_CN_ALLOW_PORT_REUSE=1.",
        );
      }
      if (driverAlreadyRunning) {
        console.error(
          `[wdio] reusing existing tauri-driver at ${driverHost}:${activeTauriDriverPort}`,
        );
        await waitTauriDriverReady(driverHost, activeTauriDriverPort);
      }
    }
    if (!driverAlreadyRunning) {
      driverProcess = spawn(driverCmd, driverArgs, {
        stdio: driverStdio,
        cwd: ROOT,
        env: driverEnv,
      });
      attachProcessDiagnostics("tauri-driver", driverProcess);
      await waitForProcessReady({
        proc: driverProcess,
        name: "tauri-driver",
        readyPromise: waitTauriDriverReady(driverHost, activeTauriDriverPort),
        detail: `Requested TAURI_DRIVER_PORT=${String(TAURI_DRIVER_PORT)} effective=${String(activeTauriDriverPort)}.`,
      });
    }
  },
  onComplete: async (exitCode) => {
    const runFailed = Number(exitCode || 0) !== 0;
    const preserveDaemonArtifacts = runFailed;
    const usesSharedCnBackend = process.platform === "darwin" && SHARED_CN_BACKEND;
    if (driverProcess) {
      driverProcess.kill();
      driverProcess = null;
    }
    if (usesSharedCnBackend) {
      try {
        await releaseSharedCnBackendLease("127.0.0.1", activeTestBackendPort);
      } catch (err) {
        console.error(`[wdio] failed to release shared CN backend lease: ${String(err)}`);
      }
    } else if (backendProcess) {
      backendProcess.kill();
      backendProcess = null;
    }
    if (ALLOW_STALE_HELPER_SWEEP && !usesSharedCnBackend) {
      // WebKit's webdriver helper can survive backend shutdown and keep stdio pipes open.
      killStaleAutomationHelpers();
    }
    if (backendLogFd !== null) {
      try {
        fs.closeSync(backendLogFd);
      } catch {
        // ignore
      }
      backendLogFd = null;
    }
    if (driverLogFd !== null) {
      try {
        fs.closeSync(driverLogFd);
      } catch {
        // ignore
      }
      driverLogFd = null;
    }
    if (backendCliAliasDir) {
      try {
        fs.rmSync(backendCliAliasDir, { recursive: true, force: true });
      } catch {
        // ignore
      }
      backendCliAliasDir = null;
    }
    if (driverCliAliasDir) {
      try {
        fs.rmSync(driverCliAliasDir, { recursive: true, force: true });
      } catch {
        // ignore
      }
      driverCliAliasDir = null;
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
        const keepInternalDir = preserveDaemonArtifacts || PRESERVE_INTERNAL_DAEMON_DATA_DIR || Boolean(INTERNAL_DAEMON_DATA_DIR_OVERRIDE);
        if (!keepInternalDir) {
          fs.rmSync(internalDaemonDataDir, { recursive: true, force: true });
        } else {
          console.error(
            `[wdio] preserving internalDaemonDataDir (test failure): ${internalDaemonDataDir}`,
          );
        }
      } catch {
        // ignore
      }
      if (!preserveDaemonArtifacts && !PRESERVE_INTERNAL_DAEMON_DATA_DIR && !INTERNAL_DAEMON_DATA_DIR_OVERRIDE) {
        internalDaemonDataDir = null;
      }
    }
  },
};

exports.__cnSharedBackendTestHooks = cnSharedBackendTestHooks;
exports.__desktopAutomationConfigTestHooks = {
  resolveMochaTimeoutMs,
};
