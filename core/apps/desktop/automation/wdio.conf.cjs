const path = require("path");
const fs = require("fs");
const crypto = require("crypto");
const { spawnSync, spawn } = require("child_process");
const os = require("os");
const { resolveBoolishFlag } = require("../../../scripts/lib/boolish.cjs");
const {
  buildLinuxAppDirLaunchEnv,
  createLinuxAppDirLaunchWrapper,
} = require("./helpers/linux_appdir_launch_env.cjs");
const { buildNonDarwinTauriDriverLaunch } = require("./helpers/tauri_driver_launch.cjs");

const resolveConfiguredPath = (rawValue) => {
  const configured = String(rawValue || "").trim();
  if (!configured) return "";
  return path.isAbsolute(configured) ? configured : path.resolve(configured);
};

const resolveVolatileRoot = () => {
  return resolveConfiguredPath(process.env.CTX_VOLATILE_ROOT)
    || path.join(os.homedir(), ".ctx", "volatile");
};

const resolveVolatileSubdir = (envName, fallbackSegments) => {
  return resolveConfiguredPath(process.env[envName])
    || path.join(resolveVolatileRoot(), ...fallbackSegments);
};

const AUTOMATION_ARTIFACTS_ROOT = resolveVolatileSubdir("CTX_VOLATILE_ARTIFACTS_DIR", ["artifacts"]);
const resolveAutomationTmpRoot = () => {
  return resolveConfiguredPath(process.env.CTX_AUTOMATION_TMPDIR || process.env.CTX_E2E_TMPDIR)
    || resolveVolatileSubdir("CTX_VOLATILE_TMPDIR", ["tmp"]);
};

const automationTmpDir = resolveAutomationTmpRoot();
fs.mkdirSync(automationTmpDir, { recursive: true });
process.env.TMPDIR = automationTmpDir;
process.env.TMP = automationTmpDir;
process.env.TEMP = automationTmpDir;
process.env.TAURI_WEBVIEW_AUTOMATION = "true";

const waitTestRunnerBackendReady = (...args) =>
  require("@crabnebula/test-runner-backend").waitTestRunnerBackendReady(...args);
const waitTauriDriverReady = (...args) =>
  require("@crabnebula/tauri-driver").waitTauriDriverReady(...args);
const resolveTestRunnerBackendCli = () => require.resolve("@crabnebula/test-runner-backend/cli.js");
const resolveTauriDriverCli = () => require.resolve("@crabnebula/tauri-driver/cli.js");

const ROOT = path.resolve(__dirname, "..");
const CORE_ROOT = path.resolve(ROOT, "..", "..");
const TAURI_TARGET_DIR = (() => {
  const configured = String(process.env.CARGO_TARGET_DIR || "").trim();
  if (configured) {
    return path.resolve(configured);
  }
  return path.resolve(ROOT, "src-tauri/target");
})();
const defaultAppPath = (() => {
  if (process.platform === "darwin") {
    return path.resolve(TAURI_TARGET_DIR, "debug/bundle/macos/ctx.app");
  }
  if (process.platform === "linux") {
    return path.resolve(TAURI_TARGET_DIR, "debug/ctx");
  }
  if (process.platform === "win32") {
    return path.resolve(TAURI_TARGET_DIR, "debug/ctx.exe");
  }
  return path.resolve(TAURI_TARGET_DIR, "debug/ctx");
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

const pickUnusedPortExcludingSync = (fallback, excludedPorts) => {
  for (let offset = 0; offset < 10; offset += 1) {
    const candidate = pickUnusedPortSync(fallback + offset);
    if (!excludedPorts.has(candidate)) {
      return candidate;
    }
  }
  for (let candidate = fallback; candidate <= 65535; candidate += 1) {
    if (!excludedPorts.has(candidate)) {
      return candidate;
    }
  }
  return fallback;
};

const DEFAULT_DRIVER_PORT = pickUnusedPortSync(4444);
const TAURI_DRIVER_PORT = parsePort(process.env.TAURI_DRIVER_PORT, DEFAULT_DRIVER_PORT);
const DEFAULT_NATIVE_DRIVER_PORT = pickUnusedPortExcludingSync(4445, new Set([TAURI_DRIVER_PORT]));
const TAURI_DRIVER_NATIVE_PORT = parsePort(
  process.env.TAURI_DRIVER_NATIVE_PORT,
  DEFAULT_NATIVE_DRIVER_PORT,
);
const TEST_BACKEND_PORT = parsePort(process.env.TAURI_TEST_BACKEND_PORT, 3000);
const FIXED_MACOS_CN_BACKEND_PORT = 3000;
const REQUESTED_MACOS_CN_BACKEND_PORT = parsePort(
  process.env.CTX_AUTOMATION_CN_BACKEND_PORT,
  FIXED_MACOS_CN_BACKEND_PORT,
);
const HAS_EXPLICIT_MACOS_CN_BACKEND_PORT =
  String(process.env.CTX_AUTOMATION_CN_BACKEND_PORT || "").trim().length > 0;
if (!String(process.env.TAURI_DRIVER_PORT || "").trim()) {
  // WDIO forks workers that reload this config; pin the chosen dynamic port for all children.
  process.env.TAURI_DRIVER_PORT = String(TAURI_DRIVER_PORT);
}
if (!String(process.env.TAURI_DRIVER_NATIVE_PORT || "").trim()) {
  process.env.TAURI_DRIVER_NATIVE_PORT = String(TAURI_DRIVER_NATIVE_PORT);
}
if (process.platform !== "darwin" && TAURI_DRIVER_NATIVE_PORT === TAURI_DRIVER_PORT) {
  throw new Error(
    `TAURI_DRIVER_NATIVE_PORT must differ from TAURI_DRIVER_PORT on ${process.platform}; both resolved to ${TAURI_DRIVER_PORT}`,
  );
}

const CTX_BIN = process.env.CTX_AUTOMATION_CTX_BIN ||
  path.resolve(ROOT, "src-tauri/bin/ctx-daemon");

const USE_EXTERNAL_DAEMON = resolveBoolishFlag(
  process.env.CTX_AUTOMATION_USE_EXTERNAL_DAEMON,
  false,
  "CTX_AUTOMATION_USE_EXTERNAL_DAEMON",
);
const ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD = resolveBoolishFlag(
  process.env.CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD,
  false,
  "CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD",
);
const SSH_NO_START_REMOTE = resolveBoolishFlag(
  process.env.CTX_AUTOMATION_SSH_NO_START_REMOTE,
  true,
  "CTX_AUTOMATION_SSH_NO_START_REMOTE",
);
const SKIP_PREP_RELEASE = resolveBoolishFlag(
  process.env.CTX_AUTOMATION_SKIP_DESKTOP_PREP_RELEASE,
  false,
  "CTX_AUTOMATION_SKIP_DESKTOP_PREP_RELEASE",
);
const SKIP_APP_BUILD = resolveBoolishFlag(
  process.env.CTX_AUTOMATION_SKIP_APP_BUILD,
  false,
  "CTX_AUTOMATION_SKIP_APP_BUILD",
);
const REMOTE_CTX_BIN = String(process.env.CTX_AUTOMATION_REMOTE_CTX_BIN || "").trim();
const REMOTE_SSH_KEY_PATH = String(
  process.env.CTX_AUTOMATION_REMOTE_SSH_KEY_PATH || process.env.CTX_UPDATER_E2E_SSH_KEY_PATH || "",
).trim();
const REMOTE_SSH_CONFIG_PATH = String(
  process.env.CTX_DESKTOP_SSH_CONFIG_PATH || process.env.CTX_AUTOMATION_REMOTE_FIXTURE_SSH_CONFIG || "",
).trim();
const SKIP_REMOTE_CTX_PROVISION = resolveBoolishFlag(
  process.env.CTX_AUTOMATION_SKIP_REMOTE_CTX_PROVISION,
  false,
  "CTX_AUTOMATION_SKIP_REMOTE_CTX_PROVISION",
);
const requestedWdioLogLevel = String(process.env.CTX_AUTOMATION_WDIO_LOG_LEVEL || "info").trim() || "info";
const REAL_AUTH_LOG_LEVEL_ENV_NAMES = Object.freeze([
  "CTX_E2E_CODEX_OAUTH_EMAIL",
  "CTX_E2E_CODEX_OAUTH_PASSWORD",
  "CTX_E2E_CODEX_OAUTH_TOTP_SECRET",
  "CTX_E2E_CURSOR_OAUTH_EMAIL",
  "CTX_E2E_CURSOR_OAUTH_PASSWORD",
  "CTX_E2E_CURSOR_EMAIL",
  "CTX_E2E_CURSOR_API_KEY",
]);
const WDIO_LOG_LEVEL_RANK = Object.freeze({
  trace: 0,
  debug: 1,
  info: 2,
  warn: 3,
  error: 4,
  silent: 5,
});
const HAS_REAL_AUTH_ENV = REAL_AUTH_LOG_LEVEL_ENV_NAMES.some((name) => String(process.env[name] || "").trim().length > 0);
const WDIO_LOG_LEVEL = (() => {
  const requestedRank = Object.prototype.hasOwnProperty.call(WDIO_LOG_LEVEL_RANK, requestedWdioLogLevel)
    ? WDIO_LOG_LEVEL_RANK[requestedWdioLogLevel]
    : WDIO_LOG_LEVEL_RANK.info;
  if (HAS_REAL_AUTH_ENV && requestedRank < WDIO_LOG_LEVEL_RANK.warn) {
    return "warn";
  }
  return requestedWdioLogLevel;
})();
const INTERNAL_DAEMON_DATA_DIR_OVERRIDE = String(
  process.env.CTX_AUTOMATION_INTERNAL_DAEMON_DATA_DIR || "",
).trim();
const PRESERVE_INTERNAL_DAEMON_DATA_DIR = resolveBoolishFlag(
  process.env.CTX_AUTOMATION_PRESERVE_INTERNAL_DAEMON_DATA_DIR,
  false,
  "CTX_AUTOMATION_PRESERVE_INTERNAL_DAEMON_DATA_DIR",
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
const resolveConnectionRetryCount = () => parsePositiveInt(
  process.env.CTX_AUTOMATION_CONNECTION_RETRY_COUNT || "3",
  3,
);
const resolveConnectionRetryTimeoutMs = () => parsePositiveInt(
  process.env.CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS || "120000",
  120000,
);
const CONNECTION_RETRY_COUNT = resolveConnectionRetryCount();
const CONNECTION_RETRY_TIMEOUT_MS = resolveConnectionRetryTimeoutMs();
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
  "sandbox",
  "host",
  "provider",
  "remote-container",
  "local-clone-sandbox",
  "local-avf-first-run",
  "local-new-host",
  "local-new-sandbox",
  "local-codex-smoke",
  "remote-container-import",
]);
const RUNS_CONTAINER_SCENARIOS = SCENARIO_FILTER.length === 0
  || SCENARIO_FILTER.some((token) => CONTAINER_SCENARIO_TOKENS.has(token));
const RUNS_REMOTE_SCENARIOS = SCENARIO_FILTER.length === 0
  || SCENARIO_FILTER.some((token) => token.startsWith("remote"));
const RUNS_REMOTE_ONLY_SCENARIOS = SCENARIO_FILTER.length > 0
  && SCENARIO_FILTER.every((token) => token.startsWith("remote"));
const ALLOW_CN_PORT_REUSE = resolveBoolishFlag(
  process.env.CTX_AUTOMATION_CN_ALLOW_PORT_REUSE,
  false,
  "CTX_AUTOMATION_CN_ALLOW_PORT_REUSE",
);
const SHARED_CN_BACKEND = resolveBoolishFlag(
  process.env.CTX_AUTOMATION_CN_SHARED_BACKEND,
  true,
  "CTX_AUTOMATION_CN_SHARED_BACKEND",
) && process.platform === "darwin";
const STOP_SHARED_CN_BACKEND_WHEN_IDLE = resolveBoolishFlag(
  process.env.CTX_AUTOMATION_CN_STOP_SHARED_BACKEND_WHEN_IDLE,
  false,
  "CTX_AUTOMATION_CN_STOP_SHARED_BACKEND_WHEN_IDLE",
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
  "TAURI_WEBVIEW_AUTOMATION",
]);
const ALLOW_STALE_HELPER_SWEEP = resolveBoolishFlag(
  process.env.CTX_AUTOMATION_ALLOW_STALE_HELPER_SWEEP,
  false,
  "CTX_AUTOMATION_ALLOW_STALE_HELPER_SWEEP",
);
const ALLOW_PREP_APP_PROCESS_SWEEP = resolveBoolishFlag(
  process.env.CTX_AUTOMATION_ALLOW_PREP_APP_PROCESS_SWEEP,
  false,
  "CTX_AUTOMATION_ALLOW_PREP_APP_PROCESS_SWEEP",
);
const SHIPPED_APP_MODE = resolveBoolishFlag(
  process.env.CTX_AUTOMATION_SHIPPED_APP,
  false,
  "CTX_AUTOMATION_SHIPPED_APP",
);
const SHIPPED_APP_BUNDLES_DIR_OVERRIDE = resolveConfiguredPath(
  process.env.CTX_AUTOMATION_SHIPPED_APP_BUNDLES_DIR,
);
const SHIPPED_APP_DAEMON_DATA_DIR_OVERRIDE = resolveConfiguredPath(
  process.env.CTX_AUTOMATION_SHIPPED_APP_DAEMON_DATA_DIR,
);

let daemonProcess = null;
let daemonDataDir = null;
let daemonPort = null;
let daemonLogPath = null;
let internalDaemonDataDir = null;
let activeTestBackendPort = TEST_BACKEND_PORT;
let activeTauriDriverPort = TAURI_DRIVER_PORT;
let activeTauriDriverNativePort = TAURI_DRIVER_NATIVE_PORT;
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

const resolveWdioApplicationPath = (appPath) => {
  const normalized = path.resolve(appPath);
  if (process.platform !== "darwin") {
    return normalized;
  }
  const bundleDir = resolveMacAppBundleDir(normalized);
  if (bundleDir) {
    return path.resolve(bundleDir, "Contents", "MacOS", "ctx");
  }
  if (normalized.endsWith(".app")) {
    return path.resolve(normalized, "Contents", "MacOS", "ctx");
  }
  return normalized;
};

const resolveAppResourcesBinPrefix = (appPath) => {
  const bundleDir = resolveMacAppBundleDir(appPath);
  if (bundleDir) {
    return path.resolve(bundleDir, "Contents", "Resources", "bin");
  }
  return path.resolve(path.dirname(appPath), "bin");
};

const resolveAppBundlesDir = (appPath) => {
  const bundleDir = resolveMacAppBundleDir(appPath);
  if (bundleDir) {
    return path.resolve(bundleDir, "Contents", "Resources", "bundles");
  }
  return "";
};

const USING_SHIPPED_APP_MODE = SHIPPED_APP_MODE;

function canonicalPath(p) {
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
}

const collectPathVariants = (candidate) => {
  const values = new Set();
  const resolved = path.resolve(String(candidate || ""));
  if (!resolved) return values;
  values.add(resolved);
  values.add(canonicalPath(resolved));
  return values;
};

const executableMatchesProcessCommand = (cmd, executablePath) =>
  cmd === executablePath || cmd.startsWith(`${executablePath} `);

const resourcePrefixMatchesProcessCommand = (cmd, resourcePrefix) =>
  cmd.startsWith(`${resourcePrefix}${path.sep}`);

const commandMatchesScopedAppProcess = (cmd, appPath) => {
  const executablePaths = Array.from(collectPathVariants(resolveAppExecutablePath(appPath)));
  if (executablePaths.some((candidate) => executableMatchesProcessCommand(cmd, candidate))) {
    return true;
  }
  const resourceBinPrefixes = Array.from(collectPathVariants(resolveAppResourcesBinPrefix(appPath)));
  return resourceBinPrefixes.some((prefix) => resourcePrefixMatchesProcessCommand(cmd, prefix));
};

const collectAutomationAppProcessSweepPaths = () => {
  const linuxAppName = process.platform === "win32" ? "ctx.exe" : "ctx";
  const shippedAppDir = path.dirname(APP_PATH);
  const candidates = new Set([
    APP_PATH,
    path.resolve(shippedAppDir, "usr", "bin", linuxAppName),
    path.resolve(shippedAppDir, "usr", "lib", "ctx", linuxAppName),
    defaultAppPath,
    path.resolve(TAURI_TARGET_DIR, "debug", linuxAppName),
    path.resolve(TAURI_TARGET_DIR, "release", linuxAppName),
    path.resolve(ROOT, "src-tauri", "target", "debug", linuxAppName),
    path.resolve(ROOT, "src-tauri", "target", "release", linuxAppName),
    path.resolve(CORE_ROOT, "target", "debug", linuxAppName),
    path.resolve(CORE_ROOT, "target", "release", linuxAppName),
  ]);
  return Array.from(candidates).filter((candidate) => String(candidate || "").trim());
};

const commandMatchesAutomationAppProcess = (cmd) =>
  collectAutomationAppProcessSweepPaths().some((candidate) => commandMatchesScopedAppProcess(cmd, candidate));

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
    // Sweep stale automation app instances and bundle-scoped helper children from prior runs.
    if (commandMatchesAutomationAppProcess(cmd)) {
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

const commandMatchesStaleAutomationHelperProcess = (cmd) =>
  /\bWebKitWebDriver\b/.test(cmd) ||
  /\bwkwebdriver\b/.test(cmd) ||
  /\bWebKitWebProcess\b/.test(cmd) ||
  /\bWebKitNetworkProcess\b/.test(cmd) ||
  /\bWebKitGPUProcess\b/.test(cmd) ||
  /\bWebKitPluginProcess\b/.test(cmd) ||
  /\bWebKitStorageProcess\b/.test(cmd) ||
  /\bWebKitWebExtension\b/.test(cmd);

const killStaleAutomationHelpers = () => {
  // WebKit child helpers can survive backend shutdown and keep pipes open.
  killProcesses((_pid, cmd) => commandMatchesStaleAutomationHelperProcess(cmd));
};

const commandHasPortArg = (cmd, port) => {
  const normalizedPort = String(port || "").trim();
  if (!normalizedPort) return false;
  return cmd.includes(`--port ${normalizedPort}`) || cmd.includes(`--port=${normalizedPort}`);
};

const commandLooksLikeTauriDriver = (cmd) =>
  /\btauri-driver\b/.test(cmd) ||
  /\bctx-tdrv-cli\b/.test(cmd) ||
  /@crabnebula[+/]tauri-driver/.test(cmd);

const killCurrentAutomationInfrastructure = () => {
  const currentTmpDir = path.resolve(automationTmpDir);
  killProcesses((_pid, cmd) =>
    (commandLooksLikeTauriDriver(cmd) && commandHasPortArg(cmd, activeTauriDriverPort)) ||
    (process.platform !== "darwin" && /\bXvfb\b/.test(cmd) && cmd.includes(currentTmpDir)),
  );
};

const ensureDesktopDevBinDir = () => {
  if (USING_SHIPPED_APP_MODE) return;
  const configured = String(process.env.CTX_DESKTOP_DEV_BIN_DIR || "").trim();
  if (configured) return;
  const defaultBinName = process.platform === "win32" ? "ctx.exe" : "ctx";
  const candidateDir = path.resolve(path.dirname(CTX_BIN));
  const candidateBin = path.join(candidateDir, defaultBinName);
  if (!fs.existsSync(candidateBin)) return;
  process.env.CTX_DESKTOP_DEV_BIN_DIR = candidateDir;
  console.error(`[wdio] CTX_DESKTOP_DEV_BIN_DIR=${candidateDir}`);
};

const avfGuestRuntimeRequiredPaths = (runtimeDir) => [
  path.join(runtimeDir, "rootfs.raw"),
  path.join(runtimeDir, "helpers", "kernel"),
  path.join(runtimeDir, "helpers", "initrd"),
  path.join(runtimeDir, "helpers", "guest-agent"),
  path.join(runtimeDir, "helpers", "egress-proxy"),
  path.join(runtimeDir, "helpers", "container-stack.tar.gz"),
];

const avfGuestRuntimeReady = (runtimeDir, fsImpl = fs) => {
  try {
    return avfGuestRuntimeRequiredPaths(runtimeDir).every((candidate) =>
      fsImpl.existsSync(candidate) && fsImpl.statSync(candidate).isFile(),
    );
  } catch {
    return false;
  }
};

const readAvfGuestRuntimeVersion = (runtimeDir, fsImpl = fs) => {
  try {
    const versionPath = path.join(runtimeDir, "version.txt");
    if (!fsImpl.existsSync(versionPath) || !fsImpl.statSync(versionPath).isFile()) {
      return "";
    }
    const contents = String(fsImpl.readFileSync(versionPath, "utf8") || "");
    const lines = contents
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean);
    if (lines.length === 0) return "";
    const explicit = lines.find((line) => line.startsWith("version="));
    if (explicit) {
      return explicit.slice("version=".length).trim();
    }
    return lines[0];
  } catch {
    return "";
  }
};

const defaultAutomationAvfGuestRuntimeDir = (platform = process.platform) => {
  return path.join(AUTOMATION_ARTIFACTS_ROOT, "ctx-desktop-e2e", "avf-linux-guest-runtime");
};

const expectedManagedAvfGuestRuntimeVersion = ({
  platform = process.platform,
  arch = process.arch,
  fsImpl = fs,
} = {}) => {
  const runtimeLockPath = path.join(resolveBundlesDir(), "runtime_lock.v2.json");
  if (!fsImpl.existsSync(runtimeLockPath)) {
    return "";
  }
  try {
    const runtimeLock = JSON.parse(fsImpl.readFileSync(runtimeLockPath, "utf8"));
    const component = findManagedComponent(
      runtimeLock,
      "runtime",
      "avf-linux-guest",
      normalizeDesktopOs(platform),
      normalizeDesktopArch(arch),
    );
    return String(component?.version || "").trim();
  } catch {
    return "";
  }
};

const managedDownloadSource = (component) => {
  const sources = Array.isArray(component?.sources) ? component.sources : [];
  return sources.find((source) => {
    const sourceType = String(source?.source_type || "").trim();
    if (!sourceType || sourceType === "local") return false;
    return String(source?.uri || "").trim() && String(source?.sha256 || "").trim();
  }) || null;
};

const resolveManagedAvfGuestRuntimeComponent = ({
  platform = process.platform,
  arch = process.arch,
  fsImpl = fs,
} = {}) => {
  const runtimeLockPath = path.join(resolveBundlesDir(), "runtime_lock.v2.json");
  if (!fsImpl.existsSync(runtimeLockPath)) {
    throw new Error(
      `runtime lock missing at ${runtimeLockPath}; run pnpm -C core desktop:prep:release`,
    );
  }
  const runtimeLock = JSON.parse(fsImpl.readFileSync(runtimeLockPath, "utf8"));
  const component = findManagedComponent(
    runtimeLock,
    "runtime",
    "avf-linux-guest",
    normalizeDesktopOs(platform),
    normalizeDesktopArch(arch),
  );
  if (!component) {
    throw new Error(
      `runtime lock missing managed AVF guest runtime source for ${normalizeDesktopOs(platform)}/${normalizeDesktopArch(arch)}; run pnpm -C core desktop:prep:release`,
    );
  }
  const rootfsSource = managedDownloadSource(component);
  if (!rootfsSource) {
    throw new Error(
      `runtime lock missing managed AVF guest runtime source for ${normalizeDesktopOs(platform)}/${normalizeDesktopArch(arch)}; run pnpm -C core desktop:prep:release`,
    );
  }
  for (const helperName of ["kernel", "initrd", "guest-agent", "egress-proxy", "container-stack"]) {
    const helper = component.helpers?.[helperName];
    if (!String(helper?.uri || "").trim() || !String(helper?.sha256 || "").trim()) {
      throw new Error(
        `runtime lock missing AVF helper metadata for ${helperName} (${normalizeDesktopOs(platform)}/${normalizeDesktopArch(arch)}); run pnpm -C core desktop:prep:release`,
      );
    }
  }
  return { component, rootfsSource };
};

const checkedSpawnSync = (spawnSyncImpl, cmd, args, options = {}) => {
  const result = spawnSyncImpl(cmd, args, { encoding: "utf8", ...options });
  if (result.status === 0) return result;
  const stderr = String(result.stderr || "").trim();
  const stdout = String(result.stdout || "").trim();
  const detail = [stderr, stdout].filter(Boolean).join("\n");
  throw new Error(`${cmd} ${args.join(" ")} failed${detail ? `: ${detail}` : ""}`);
};

const sha256File = (filePath, { spawnSyncImpl = spawnSync } = {}) => {
  const result = checkedSpawnSync(spawnSyncImpl, "shasum", ["-a", "256", filePath]);
  return String(result.stdout || "")
    .trim()
    .split(/\s+/)[0] || "";
};

const materializeManagedAvfGuestRuntime = ({
  runtimeDir,
  component,
  rootfsSource,
  fsImpl = fs,
  spawnSyncImpl = spawnSync,
  log = console.error,
} = {}) => {
  const tempDir = `${runtimeDir}.tmp-${process.pid}-${Date.now().toString(36)}`;
  fsImpl.rmSync(tempDir, { recursive: true, force: true });
  fsImpl.mkdirSync(path.join(tempDir, "helpers"), { recursive: true });
  const rootfsArchivePath = path.join(tempDir, "rootfs.raw.zst");
  const rootfsPath = path.join(tempDir, "rootfs.raw");
  const downloadPlan = [
    { uri: String(rootfsSource.uri || "").trim(), sha256: String(rootfsSource.sha256 || "").trim(), path: rootfsArchivePath },
    { uri: String(component.helpers.kernel.uri || "").trim(), sha256: String(component.helpers.kernel.sha256 || "").trim(), path: path.join(tempDir, "helpers", "kernel") },
    { uri: String(component.helpers.initrd.uri || "").trim(), sha256: String(component.helpers.initrd.sha256 || "").trim(), path: path.join(tempDir, "helpers", "initrd") },
    { uri: String(component.helpers["guest-agent"].uri || "").trim(), sha256: String(component.helpers["guest-agent"].sha256 || "").trim(), path: path.join(tempDir, "helpers", "guest-agent") },
    { uri: String(component.helpers["egress-proxy"].uri || "").trim(), sha256: String(component.helpers["egress-proxy"].sha256 || "").trim(), path: path.join(tempDir, "helpers", "egress-proxy") },
    { uri: String(component.helpers["container-stack"].uri || "").trim(), sha256: String(component.helpers["container-stack"].sha256 || "").trim(), path: path.join(tempDir, "helpers", "container-stack.tar.gz") },
  ];
  try {
    for (const artifact of downloadPlan) {
      checkedSpawnSync(spawnSyncImpl, "curl", ["-fsSL", "--retry", "4", "-o", artifact.path, artifact.uri], {
        stdio: "inherit",
      });
      const actualSha = sha256File(artifact.path, { spawnSyncImpl });
      if (actualSha !== artifact.sha256) {
        throw new Error(
          `downloaded AVF runtime artifact sha256 mismatch for ${artifact.path}: expected ${artifact.sha256}, found ${actualSha || "missing"}`,
        );
      }
    }
    checkedSpawnSync(spawnSyncImpl, "zstd", ["-d", "-f", rootfsArchivePath, "-o", rootfsPath], {
      stdio: "inherit",
    });
    fsImpl.rmSync(rootfsArchivePath, { force: true });
    fsImpl.writeFileSync(
      path.join(tempDir, "version.txt"),
      `version=${String(component.version || "").trim()}\nmanaged-source=runtime_lock\n`,
      "utf8",
    );
    fsImpl.rmSync(runtimeDir, { recursive: true, force: true });
    fsImpl.renameSync(tempDir, runtimeDir);
    log(`[wdio] CTX_AVF_LINUX_GUEST_RUNTIME_DIR materialized from runtime lock ${component.version}`);
  } catch (error) {
    fsImpl.rmSync(tempDir, { recursive: true, force: true });
    throw error;
  }
};

const ensureAutomationAvfLinuxGuestRuntime = ({
  platform = process.platform,
  arch = process.arch,
  runsContainerScenarios = RUNS_CONTAINER_SCENARIOS,
  env = process.env,
  fsImpl = fs,
  spawnSyncImpl = spawnSync,
  coreRoot = CORE_ROOT,
  log = console.error,
} = {}) => {
  if (platform !== "darwin" || !runsContainerScenarios) {
    return null;
  }

  const configured = String(
    env.CTX_AVF_LINUX_GUEST_RUNTIME_DIR || env.CTX_AUTOMATION_AVF_LINUX_GUEST_RUNTIME_DIR || "",
  ).trim();
  const runtimeDir = path.resolve(configured || defaultAutomationAvfGuestRuntimeDir(platform));
  const { component, rootfsSource } = resolveManagedAvfGuestRuntimeComponent({ platform, arch, fsImpl });
  const expectedVersion = String(component.version || "").trim();
  const runtimeVersion = readAvfGuestRuntimeVersion(runtimeDir, fsImpl);
  const runtimeMatchesExpectedVersion = !expectedVersion || runtimeVersion === expectedVersion;
  if (avfGuestRuntimeReady(runtimeDir, fsImpl) && runtimeMatchesExpectedVersion) {
    env.CTX_AVF_LINUX_GUEST_RUNTIME_DIR = runtimeDir;
    log(`[wdio] CTX_AVF_LINUX_GUEST_RUNTIME_DIR=${runtimeDir}`);
    return runtimeDir;
  }
  if (avfGuestRuntimeReady(runtimeDir, fsImpl) && !runtimeMatchesExpectedVersion) {
    log(
      `[wdio] refreshing stale AVF Linux guest runtime at ${runtimeDir} (expected ${expectedVersion || "unknown"}, found ${runtimeVersion || "missing"})`,
    );
  }

  fsImpl.mkdirSync(path.dirname(runtimeDir), { recursive: true });
  materializeManagedAvfGuestRuntime({
    runtimeDir,
    component,
    rootfsSource,
    fsImpl,
    spawnSyncImpl,
    log,
  });
  if (!avfGuestRuntimeReady(runtimeDir, fsImpl)) {
    throw new Error(
      `prepared AVF Linux guest runtime is incomplete at ${runtimeDir}; expected ${avfGuestRuntimeRequiredPaths(runtimeDir).join(", ")}`,
    );
  }
  const preparedVersion = readAvfGuestRuntimeVersion(runtimeDir, fsImpl);
  if (expectedVersion && preparedVersion !== expectedVersion) {
    throw new Error(
      `prepared AVF Linux guest runtime version mismatch at ${runtimeDir}; expected ${expectedVersion}, found ${preparedVersion || "missing"}`,
    );
  }
  env.CTX_AVF_LINUX_GUEST_RUNTIME_DIR = runtimeDir;
  log(`[wdio] CTX_AVF_LINUX_GUEST_RUNTIME_DIR=${runtimeDir}`);
  return runtimeDir;
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

const buildDesktopAppLaunchEnv = () => {
  const env = {
    TAURI_WEBVIEW_AUTOMATION: "true",
  };
  for (const key of [
    "APPDIR",
    "APPIMAGE",
    "APPIMAGE_EXTRACT_AND_RUN",
    "ARGV0",
    "CTX_APPIMAGE_PATH",
    "DISPLAY",
    "HOME",
    "TMP",
    "TEMP",
    "TMPDIR",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_RUNTIME_DIR",
  ]) {
    const value = String(process.env[key] || "").trim();
    if (value) {
      env[key] = value;
    }
  }
  Object.assign(env, buildLinuxAppDirLaunchEnv({
    appPath: APP_PATH,
    env: {
      ...process.env,
      ...env,
    },
  }));
  if (SSH_NO_START_REMOTE) {
    env.CTX_DESKTOP_SSH_NO_START_REMOTE = "1";
    env.CTX_DESKTOP_SSH_START_REMOTE = "0";
  } else {
    env.CTX_DESKTOP_SSH_NO_START_REMOTE = "0";
    env.CTX_DESKTOP_SSH_START_REMOTE = "1";
  }
  if (USING_SHIPPED_APP_MODE) {
    if (SHIPPED_APP_BUNDLES_DIR_OVERRIDE) {
      env.CTX_BUNDLE_DIR = SHIPPED_APP_BUNDLES_DIR_OVERRIDE;
    }
    if (!USE_EXTERNAL_DAEMON && SHIPPED_APP_DAEMON_DATA_DIR_OVERRIDE) {
      env.CTX_DESKTOP_DAEMON_DATA_DIR = SHIPPED_APP_DAEMON_DATA_DIR_OVERRIDE;
    }
  }
  return env;
};

const createDesktopAppLaunchWrapper = (appExecutablePath, env) => {
  if (process.platform === "linux") {
    return createLinuxAppDirLaunchWrapper({
      appPath: appExecutablePath,
      env,
      wrapperDir: path.join(automationTmpDir, "desktop-app-launchers"),
    });
  }
  const entries = Object.entries(env).filter(([, value]) => String(value || "").trim());
  if (process.platform === "win32" || entries.length === 0) {
    return appExecutablePath;
  }
  const signature = crypto
    .createHash("sha256")
    .update(JSON.stringify({ appExecutablePath, env }))
    .digest("hex")
    .slice(0, 16);
  const wrapperDir = path.join(automationTmpDir, "desktop-app-launchers");
  fs.mkdirSync(wrapperDir, { recursive: true });
  const wrapperPath = path.join(wrapperDir, `ctx-app-${signature}.sh`);
  const lines = [
    "#!/bin/sh",
    "set -eu",
    ...entries.map(([key, value]) => `export ${key}=${shellQuote(value)}`),
    "if [ -n \"${CTX_AUTOMATION_APP_LAUNCH_LOG:-}\" ]; then",
    "  mkdir -p \"$(dirname \"$CTX_AUTOMATION_APP_LAUNCH_LOG\")\"",
    "  printf '%s\\n' \"launching desktop app\" >> \"$CTX_AUTOMATION_APP_LAUNCH_LOG\"",
    "  printf 'launch env TAURI_WEBVIEW_AUTOMATION=%s APPDIR=%s APPIMAGE=%s ARGV0=%s CTX_APPIMAGE_PATH=%s DISPLAY=%s XDG_RUNTIME_DIR=%s HOME=%s\\n' \"${TAURI_WEBVIEW_AUTOMATION:-}\" \"${APPDIR:-}\" \"${APPIMAGE:-}\" \"${ARGV0:-}\" \"${CTX_APPIMAGE_PATH:-}\" \"${DISPLAY:-}\" \"${XDG_RUNTIME_DIR:-}\" \"${HOME:-}\" >> \"$CTX_AUTOMATION_APP_LAUNCH_LOG\"",
    `  exec ${shellQuote(appExecutablePath)} "$@" >> "$CTX_AUTOMATION_APP_LAUNCH_LOG" 2>&1`,
    "fi",
    `exec ${shellQuote(appExecutablePath)} "$@"`,
    "",
  ];
  fs.writeFileSync(wrapperPath, lines.join("\n"), { mode: 0o700 });
  fs.chmodSync(wrapperPath, 0o700);
  return wrapperPath;
};

const resolveAutomationApplicationPath = (appPath) => {
  const appExecutablePath = resolveWdioApplicationPath(appPath);
  return createDesktopAppLaunchWrapper(appExecutablePath, DESKTOP_APP_LAUNCH_ENV);
};

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

const hasManagedDownloadSource = (component) => {
  const sources = Array.isArray(component?.sources) ? component.sources : [];
  return sources.some((source) => {
    const sourceType = String(source?.source_type || "").trim();
    if (!sourceType || sourceType === "local") return false;
    return String(source?.uri || "").trim() && String(source?.sha256 || "").trim();
  });
};

const findManagedComponent = (lock, kind, id, osValue, archValue) => {
  const components = Array.isArray(lock?.components) ? lock.components : [];
  return components.find((component) =>
    component
    && component.kind === kind
    && component.id === id
    && component.os === osValue
    && component.arch === archValue
    && String(component.variant || "default").trim() === "default"
  );
};

const assertManagedAvfRuntimeComponent = (lock, hostOs, hostArch) => {
  const component = findManagedComponent(lock, "runtime", "avf-linux-guest", hostOs, hostArch);
  if (!component || !hasManagedDownloadSource(component)) {
    throw new Error(
      `runtime lock missing managed AVF guest runtime source for ${hostOs}/${hostArch}; run pnpm -C core desktop:prep:release`,
    );
  }
  const helpers = component.helpers || {};
  for (const helperName of ["kernel", "initrd", "guest-agent", "egress-proxy", "container-stack"]) {
    const helper = helpers[helperName];
    if (!String(helper?.uri || "").trim() || !String(helper?.sha256 || "").trim()) {
      throw new Error(
        `runtime lock missing AVF helper metadata for ${helperName} (${hostOs}/${hostArch}); run pnpm -C core desktop:prep:release`,
      );
    }
  }
};

const assertManagedHarnessImageComponent = (lock, target) => {
  const component = findManagedComponent(lock, "image", "ctx-harness", target.os, target.arch);
  if (!component || !hasManagedDownloadSource(component)) {
    throw new Error(
      `runtime lock missing managed harness image source for ${target.os}/${target.arch}; run pnpm -C core desktop:prep:release`,
    );
  }
};

const normalizeDesktopOs = (platform = process.platform) => {
  if (platform === "darwin" || platform === "macos") return "macos";
  if (platform === "win32" || platform === "windows") return "windows";
  return "linux";
};

const normalizeDesktopArch = (arch = process.arch) => {
  if (arch === "arm64" || arch === "aarch64") return "aarch64";
  if (arch === "x64" || arch === "x86_64") return "x86_64";
  return arch;
};

const desktopOs = () => normalizeDesktopOs(process.platform);

const desktopArch = () => normalizeDesktopArch(process.arch);

const normalizeTargetToken = (raw, hostValue) => {
  const trimmed = String(raw || "").trim();
  if (!trimmed) return null;
  return trimmed === "host" ? hostValue : trimmed;
};

const requiredTargetsFromRuntimeLock = (lock, kind, hostOs, hostArch, fallbackTargets) => {
  const configured = Array.isArray(lock?.required?.targets?.[kind]) ? lock.required.targets[kind] : [];
  if (configured.length === 0) {
    return fallbackTargets;
  }
  const seen = new Set();
  const targets = [];
  for (const raw of configured) {
    const [rawOs, rawArch] = String(raw || "").split("/");
    if (!rawOs || !rawArch) continue;
    const osValue = normalizeTargetToken(rawOs, hostOs);
    const archValue = normalizeTargetToken(rawArch, hostArch);
    if (!osValue || !archValue) continue;
    const key = `${osValue}/${archValue}`;
    if (seen.has(key)) continue;
    seen.add(key);
    targets.push({ os: osValue, arch: archValue });
  }
  return targets.length > 0 ? targets : fallbackTargets;
};

const resolveBundlesDir = () => {
  const configured = String(process.env.CTX_BUNDLE_DIR || "").trim();
  if (configured) {
    return path.resolve(configured);
  }
  if (USING_SHIPPED_APP_MODE) {
    if (SHIPPED_APP_BUNDLES_DIR_OVERRIDE) {
      return SHIPPED_APP_BUNDLES_DIR_OVERRIDE;
    }
    return resolveAppBundlesDir(APP_PATH);
  }
  return BUNDLES_DIR;
};

const ensureBundledContainerAssets = (options = {}) => {
  const bundlesDir = resolveBundlesDir();
  const manifestPath = path.join(bundlesDir, "manifest.json");
  const runtimeLockPath = path.join(bundlesDir, "runtime_lock.v2.json");
  if (!fs.existsSync(manifestPath)) {
    throw new Error(
      `bundled manifest missing at ${manifestPath}; run pnpm -C core desktop:prep:release`,
    );
  }
  if (!fs.existsSync(runtimeLockPath)) {
    throw new Error(
      `runtime lock missing at ${runtimeLockPath}; run pnpm -C core desktop:prep:release`,
    );
  }
  const manifest = readJson(manifestPath);
  const runtimeLock = readJson(runtimeLockPath);
  const runtimes = Array.isArray(manifest?.runtimes) ? manifest.runtimes : [];
  const images = Array.isArray(manifest?.images)
    ? manifest.images
    : Array.isArray(manifest?.harness_images)
      ? manifest.harness_images
      : [];

  const hostOs = normalizeDesktopOs(options.platform);
  const hostArch = normalizeDesktopArch(options.arch);
  const requiresBundledAvfRuntime = hostOs === "macos" && hostArch === "aarch64";
  const allowManagedAvfRuntimeWithoutLocalPayload =
    USING_SHIPPED_APP_MODE || ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD;
  if (requiresBundledAvfRuntime) {
    const avfGuestRuntime = runtimes.find((entry) =>
      entry
      && entry.id === "avf-linux-guest"
      && entry.os === hostOs
      && entry.arch === hostArch
      && typeof entry.root === "string"
      && entry.root.trim().length > 0
      && typeof entry.bin === "string"
      && entry.bin.trim().length > 0
    );
    if (avfGuestRuntime) {
      const runtimeRoot = path.join(bundlesDir, avfGuestRuntime.root);
      if (fs.existsSync(runtimeRoot) && fs.statSync(runtimeRoot).isDirectory()) {
        const requiredPaths = [
          path.join(runtimeRoot, avfGuestRuntime.bin),
          path.join(runtimeRoot, "helpers", "kernel"),
          path.join(runtimeRoot, "helpers", "initrd"),
          path.join(runtimeRoot, "helpers", "guest-agent"),
          path.join(runtimeRoot, "helpers", "egress-proxy"),
          path.join(runtimeRoot, "helpers", "container-stack.tar.gz"),
        ];
        const missingRequiredPath = requiredPaths.find((requiredPath) => !fs.existsSync(requiredPath));
        if (missingRequiredPath) {
          if (allowManagedAvfRuntimeWithoutLocalPayload) {
            assertManagedAvfRuntimeComponent(runtimeLock, hostOs, hostArch);
            return;
          }
          throw new Error(
            `bundled AVF guest runtime asset missing at ${missingRequiredPath}; run pnpm -C core desktop:prep:release`,
          );
        }
      } else if (allowManagedAvfRuntimeWithoutLocalPayload) {
        assertManagedAvfRuntimeComponent(runtimeLock, hostOs, hostArch);
      } else {
        throw new Error(
          `bundled AVF guest runtime directory missing at ${runtimeRoot}; run pnpm -C core desktop:prep:release`,
        );
      }
    } else {
      assertManagedAvfRuntimeComponent(runtimeLock, hostOs, hostArch);
    }
  }

  const requiredHarnessTargets = requiredTargetsFromRuntimeLock(
    runtimeLock,
    "image",
    hostOs,
    hostArch,
    [{ os: "linux", arch: hostArch }],
  ).filter((target) => target.os === "linux");
  for (const target of requiredHarnessTargets) {
    const harnessImage = images.find((entry) =>
      entry
      && entry.id === "ctx-harness"
      && entry.os === target.os
      && entry.arch === target.arch
      && typeof entry.tar === "string"
      && entry.tar.trim().length > 0
    );
    if (harnessImage) {
      const harnessImageTar = path.join(bundlesDir, harnessImage.tar);
      if (!fs.existsSync(harnessImageTar)) {
        throw new Error(
          `bundled harness image tar missing at ${harnessImageTar}; run pnpm -C core desktop:prep:release`,
        );
      }
      continue;
    }
    assertManagedHarnessImageComponent(runtimeLock, target);
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
  daemonDataDir = fs.mkdtempSync(path.join(automationTmpDir, `ctx-desktop-e2e-${daemonPort}-`));
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

const createAppBuildInvocation = () => {
  const desktopTauriEntryPath = path.resolve(CORE_ROOT, "scripts/desktop_tauri_entry.cjs");
  const appPathLooksLikeBundle = process.platform === "darwin" && APP_PATH.endsWith(".app");
  const darwinBundles = String(process.env.CTX_AUTOMATION_TAURI_BUNDLES || "app").trim() || "app";
  const args = appPathLooksLikeBundle
    ? [desktopTauriEntryPath, "build", "--debug", "--bundles", darwinBundles, "--", "--features", "automation"]
    : [desktopTauriEntryPath, "build", "--debug", "--no-bundle", "--", "--features", "automation"];
  const env = {
    ...process.env,
    CARGO_INCREMENTAL: String(process.env.CARGO_INCREMENTAL || "0").trim() || "0",
    CTX_DESKTOP_SYNC_BUNDLES: String(process.env.CTX_DESKTOP_SYNC_BUNDLES || "0").trim() || "0",
  };
  // EXCEPTION: WDIO source app builds run after cache-managed prep steps and
  // must not inherit host sccache socket state. Deep Buildkite workspaces can
  // make inherited Unix socket paths exceed sockaddr_un.sun_path before the
  // app is even launched, which hides the real desktop acceptance signal.
  for (const key of [
    "RUSTC_WRAPPER",
    "SCCACHE_CONF",
    "SCCACHE_DIR",
    "SCCACHE_ERROR_LOG",
    "SCCACHE_LOG",
    "SCCACHE_PATH",
    "SCCACHE_SERVER_UDS",
    "SCCACHE_START_SERVER",
  ]) {
    delete env[key];
  }
  return {
    command: process.execPath,
    args,
    cwd: CORE_ROOT,
    env,
  };
};

const buildAppIfMissing = () => {
  if (USING_SHIPPED_APP_MODE) return;
  if (SKIP_APP_BUILD) return;
  const buildInvocation = createAppBuildInvocation();
  const result = spawnSync(
    buildInvocation.command,
    buildInvocation.args,
    {
      stdio: "inherit",
      cwd: buildInvocation.cwd,
      env: buildInvocation.env,
    },
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
  const backendAlias = createCliAlias(resolveTestRunnerBackendCli(), "ctx-cnb-cli");
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
        TAURI_TEST_BACKEND_PORT: String(port),
        TEST_RUNNER_BACKEND_PORT: String(port),
        CTX_AUTOMATION_CN_BACKEND_PORT: String(port),
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

    const existingStateTracked = Object.keys(existingState).length > 0;
    const existingSignatureTrusted = existingState.launchEnvSignatureSource === "spawn";
    const launchEnvNeedsRestart = existingStateTracked && Boolean(launchEnvSignature) && (
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
      } else {
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
    }

    if (portOpen && !existingPidAlive) {
      throw new Error(
        `[wdio] shared test-runner-backend port ${port} is already in use at ${host}, ` +
          `but ${CN_BACKEND_STATE_FILE} does not describe a live backend for this shared state dir. ` +
          "Refusing to attach to an unowned backend; stop the stale backend or reuse the matching " +
          "CTX_AUTOMATION_CN_BACKEND_STATE_DIR.",
      );
    }

    writeJsonFileAtomic(leaseFile, {
      leaseId,
      pid: process.pid,
      host,
      port,
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
    });

    if (existingPidAlive) {
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
  acquireSharedCnBackendLease,
  writeJsonFileAtomic,
  removeFileIfExists,
  releaseSharedCnBackendLease,
  setCurrentLeaseId: (leaseId) => {
    cnBackendLeaseId = leaseId;
  },
};

const DESKTOP_APP_LAUNCH_ENV = buildDesktopAppLaunchEnv();
const WDIO_APPLICATION_PATH = resolveAutomationApplicationPath(APP_PATH);

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
  connectionRetryCount: CONNECTION_RETRY_COUNT,
  connectionRetryTimeout: CONNECTION_RETRY_TIMEOUT_MS,
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
        application: WDIO_APPLICATION_PATH,
      },
    },
  ],
  port: activeTauriDriverPort,
  path: "/",
  automationProtocol: "webdriver",
  beforeSession: () => {
    process.env.CTX_AUTOMATION_WORKSPACE_PATH = WORKSPACE_PATH;
  },
  before: async (_capabilities, _specs, browser) => {
    if (process.platform === "darwin") {
      await browser.getWindowHandle();
      if (!process.env.CTX_AUTOMATION_APP_OPEN_OBSERVED_AT) {
        process.env.CTX_AUTOMATION_APP_OPEN_OBSERVED_AT = new Date().toISOString();
      }
    }
  },
  onPrepare: async () => {
    // Debug breadcrumb for remote-start behavior in automation logs.
    const isDarwin = process.platform === "darwin";
    console.error(
      `[wdio] CTX_AUTOMATION_SSH_NO_START_REMOTE=${String(process.env.CTX_AUTOMATION_SSH_NO_START_REMOTE || "<unset>")} SSH_NO_START_REMOTE=${String(SSH_NO_START_REMOTE)}`,
    );
    console.error(`[wdio] app path=${APP_PATH}`);
    console.error(`[wdio] WebDriver application path=${WDIO_APPLICATION_PATH}`);
    activeTauriDriverPort = TAURI_DRIVER_PORT;
    activeTauriDriverNativePort = TAURI_DRIVER_NATIVE_PORT;
    console.error(
      `[wdio] ports driver(requested)=${String(TAURI_DRIVER_PORT)} driver(effective)=${String(activeTauriDriverPort)} native_driver(requested)=${String(TAURI_DRIVER_NATIVE_PORT)} native_driver(effective)=${String(activeTauriDriverNativePort)} backend(requested)=${isDarwin ? String(HAS_EXPLICIT_MACOS_CN_BACKEND_PORT ? REQUESTED_MACOS_CN_BACKEND_PORT : FIXED_MACOS_CN_BACKEND_PORT) : String(TEST_BACKEND_PORT)} backend(effective)=${isDarwin ? String(FIXED_MACOS_CN_BACKEND_PORT) : String(TEST_BACKEND_PORT)}`,
    );
    if (isDarwin && !process.env.CN_API_KEY) {
      throw new Error(
        "CN_API_KEY is required for CrabNebula WebDriver on macOS. " +
          "Load it from Infisical in core/ (core/.infisical.json), or run `pnpm -C core verify:desktop-smoke` which loads Infisical by default.",
      );
    }
    if (USING_SHIPPED_APP_MODE) {
      if (USE_EXTERNAL_DAEMON) {
        throw new Error("CTX_AUTOMATION_SHIPPED_APP=1 does not allow CTX_AUTOMATION_USE_EXTERNAL_DAEMON=1.");
      }
      if (INTERNAL_DAEMON_DATA_DIR_OVERRIDE) {
        throw new Error(
          "CTX_AUTOMATION_SHIPPED_APP=1 does not allow CTX_AUTOMATION_INTERNAL_DAEMON_DATA_DIR; the shipped app must use the real ~/.ctx data dir.",
        );
      }
      if (String(process.env.CTX_BUNDLE_DIR || "").trim()) {
        throw new Error(
          "CTX_AUTOMATION_SHIPPED_APP=1 does not allow CTX_BUNDLE_DIR overrides; use CTX_AUTOMATION_SHIPPED_APP_BUNDLES_DIR when the shipped app bundles must be provided explicitly.",
        );
      }
    }
    if (ALLOW_PREP_APP_PROCESS_SWEEP) {
      // Automation builds disable single-instance mode; only use global app sweeps when explicitly requested.
      killExistingAppProcesses();
    }
    if (ALLOW_STALE_HELPER_SWEEP && !(isDarwin && SHARED_CN_BACKEND)) {
      killStaleAutomationHelpers();
    }
    stopStaleSystemdScope();

    if (!SKIP_PREP_RELEASE && !USING_SHIPPED_APP_MODE) {
      ensureAutomationAvfLinuxGuestRuntime();
    }

    // Container-mode provider smoke needs a fully-bundled release-style resource set
    // (Linux provider binaries + harness image tars). Keep the app build in debug mode
    // for the automation plugin, but sync release resources.
    if (!SKIP_PREP_RELEASE && !USING_SHIPPED_APP_MODE) {
      const prepEnv = { ...process.env };
      if (
        process.platform === "darwin"
        && process.arch === "arm64"
        && RUNS_REMOTE_ONLY_SCENARIOS
        && !String(prepEnv.CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD || "").trim()
      ) {
        prepEnv.CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD = "1";
      }
      if (!RUNS_REMOTE_SCENARIOS && !String(prepEnv.CTX_BUNDLE_REMOTE_DAEMONS || "").trim()) {
        prepEnv.CTX_BUNDLE_REMOTE_DAEMONS = "0";
      }
      if (
        !RUNS_CONTAINER_SCENARIOS
        && !RUNS_REMOTE_SCENARIOS
        && !String(prepEnv.CTX_BUNDLE_LINUX_CTX_MCP_RUNTIME || "").trim()
      ) {
        prepEnv.CTX_BUNDLE_LINUX_CTX_MCP_RUNTIME = "0";
      }
      if (
        process.platform === "darwin"
        && process.arch === "arm64"
        && RUNS_CONTAINER_SCENARIOS
        && String(prepEnv.CTX_AVF_LINUX_GUEST_RUNTIME_DIR || "").trim()
        && !String(prepEnv.CTX_DESKTOP_STAGE_AVF_LINUX_GUEST_RUNTIME || "").trim()
      ) {
        prepEnv.CTX_DESKTOP_STAGE_AVF_LINUX_GUEST_RUNTIME = "0";
      }
      delete prepEnv.NODE_OPTIONS;
      const prepRelease = spawnSync("pnpm", ["-C", CORE_ROOT, "desktop:prep:release"], {
        stdio: "inherit",
        cwd: ROOT,
        shell: true,
        env: prepEnv,
      });
      if (prepRelease.status !== 0) {
        throw new Error("pnpm -C core desktop:prep:release failed");
      }
    }
    if (!process.env.CTX_BUNDLE_DIR && !USING_SHIPPED_APP_MODE) {
      process.env.CTX_BUNDLE_DIR = BUNDLES_DIR;
    }
    if (USING_SHIPPED_APP_MODE) {
      if (RUNS_CONTAINER_SCENARIOS && !SHIPPED_APP_BUNDLES_DIR_OVERRIDE && !resolveAppBundlesDir(APP_PATH)) {
        throw new Error(
          "CTX_AUTOMATION_SHIPPED_APP=1 requires CTX_AUTOMATION_SHIPPED_APP_BUNDLES_DIR for Linux/Windows container scenarios so automation can validate the installed bundle resources.",
        );
      }
      if (SHIPPED_APP_BUNDLES_DIR_OVERRIDE) {
        process.env.CTX_BUNDLE_DIR = SHIPPED_APP_BUNDLES_DIR_OVERRIDE;
      } else {
        delete process.env.CTX_BUNDLE_DIR;
      }
      delete process.env.CTX_DESKTOP_DEV_BIN_DIR;
      delete process.env.CTX_DESKTOP_START_PATH;
      console.error(`[wdio] shipped-app mode using bundled resources at ${resolveBundlesDir()}`);
    }
    // Debug desktop binaries resolve local daemon executables from this directory.
    ensureDesktopDevBinDir();
    if (RUNS_CONTAINER_SCENARIOS) {
      ensureBundledContainerAssets();
    }

    // Launch the app directly into the wizard route to reduce test flakiness.
    if (!USING_SHIPPED_APP_MODE) {
      process.env.CTX_DESKTOP_START_PATH = "/workspace-setup";
    }
    const codexOauthFlowEnabled = resolveBoolishFlag(
      process.env.CTX_AUTOMATION_CODEX_OAUTH_ENABLE,
      false,
      "CTX_AUTOMATION_CODEX_OAUTH_ENABLE",
    );
    process.env.CTX_SEED_CODEX_AUTH_FROM_HOST = process.env.CTX_SEED_CODEX_AUTH_FROM_HOST
      || (codexOauthFlowEnabled ? "0" : "1");
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
      if (USING_SHIPPED_APP_MODE) {
        if (SHIPPED_APP_DAEMON_DATA_DIR_OVERRIDE) {
          fs.mkdirSync(SHIPPED_APP_DAEMON_DATA_DIR_OVERRIDE, { recursive: true });
          internalDaemonDataDir = canonicalPath(SHIPPED_APP_DAEMON_DATA_DIR_OVERRIDE);
        } else {
          delete process.env.CTX_DESKTOP_DAEMON_DATA_DIR;
          internalDaemonDataDir = null;
        }
      } else if (INTERNAL_DAEMON_DATA_DIR_OVERRIDE) {
        internalDaemonDataDir = path.resolve(INTERNAL_DAEMON_DATA_DIR_OVERRIDE);
        fs.mkdirSync(internalDaemonDataDir, { recursive: true });
        internalDaemonDataDir = canonicalPath(internalDaemonDataDir);
      } else {
        internalDaemonDataDir = fs.mkdtempSync(path.join(automationTmpDir, "ctx-desktop-e2e-app-daemon-"));
        internalDaemonDataDir = canonicalPath(internalDaemonDataDir);
      }
      if (internalDaemonDataDir) {
        process.env.CTX_DESKTOP_DAEMON_DATA_DIR = internalDaemonDataDir;
      } else {
        delete process.env.CTX_DESKTOP_DAEMON_DATA_DIR;
      }
    }

    buildAppIfMissing();
    // Validate executable/bundle after optional build so clean machines can self-bootstrap.
    ensureAppExecutable();

    if (isDarwin) {
      activeTestBackendPort = FIXED_MACOS_CN_BACKEND_PORT;
      if (TEST_BACKEND_PORT !== FIXED_MACOS_CN_BACKEND_PORT) {
        console.error(
          `[wdio] macOS backend currently binds ${FIXED_MACOS_CN_BACKEND_PORT}; ignoring requested TAURI_TEST_BACKEND_PORT=${TEST_BACKEND_PORT}`,
        );
      }
      if (
        HAS_EXPLICIT_MACOS_CN_BACKEND_PORT
        && REQUESTED_MACOS_CN_BACKEND_PORT !== FIXED_MACOS_CN_BACKEND_PORT
      ) {
        console.error(
          `[wdio] CrabNebula backend currently binds fixed port ${FIXED_MACOS_CN_BACKEND_PORT} on macOS; ignoring requested CTX_AUTOMATION_CN_BACKEND_PORT=${REQUESTED_MACOS_CN_BACKEND_PORT}`,
        );
      }
      if (!SHARED_CN_BACKEND) {
        console.error(
          `[wdio] using dedicated CrabNebula backend on fixed macOS port ${activeTestBackendPort} for non-shared automation`,
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
      ...DESKTOP_APP_LAUNCH_ENV,
      TAURI_DRIVER_PORT: String(activeTauriDriverPort),
      TAURI_DRIVER_NATIVE_PORT: String(activeTauriDriverNativePort),
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
      const driverAlias = createCliAlias(resolveTauriDriverCli(), "ctx-tdrv-cli");
      driverCliAliasDir = driverAlias.aliasDir;
      driverCmd = process.execPath;
      driverArgs = [driverAlias.cliPath, "--port", String(activeTauriDriverPort)];
    } else {
      const nonDarwinLaunch = buildNonDarwinTauriDriverLaunch({
        platform: process.platform,
        hasDisplay: Boolean(process.env.DISPLAY),
        port: activeTauriDriverPort,
        nativePort: activeTauriDriverNativePort,
      });
      driverCmd = nonDarwinLaunch.command;
      driverArgs = nonDarwinLaunch.args;
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
        detail: `Requested TAURI_DRIVER_PORT=${String(TAURI_DRIVER_PORT)} effective=${String(activeTauriDriverPort)} native=${String(activeTauriDriverNativePort)}.`,
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
    killCurrentAutomationInfrastructure();
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
  ensureAutomationAvfLinuxGuestRuntime,
  ensureBundledContainerAssets,
  expectedManagedAvfGuestRuntimeVersion,
  readAvfGuestRuntimeVersion,
  resolveMochaTimeoutMs,
  resolveConnectionRetryCount,
  resolveConnectionRetryTimeoutMs,
  commandMatchesScopedAppProcess,
  collectAutomationAppProcessSweepPaths,
  commandMatchesAutomationAppProcess,
  commandMatchesStaleAutomationHelperProcess,
  createAppBuildInvocation,
  buildAppIfMissing,
};
