#!/usr/bin/env node
import { cpSync, createWriteStream, existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, readdirSync, realpathSync, renameSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { spawn, spawnSync } from "node:child_process";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import net from "node:net";
import os from "node:os";

import { startDemoRelay } from "./demo-relay/index.mjs";
import { bootstrapDemoFixture } from "./demo_fixture_bootstrap.mjs";
import { configureProviderEndpoint } from "./demo_configure_provider_endpoint.mjs";
import {
  api,
  getProviderStatus,
  installProviderAndWait,
  readDaemonAuth,
  sleep,
  waitFor,
  waitForFile,
  waitForHttpOk,
} from "./demo_lib.mjs";
import { buildClickScenario, buildPromptScenario, screenPointFromProbe } from "./demo_desktop_probe.mjs";

const require = createRequire(import.meta.url);
const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(SCRIPT_DIR, "../../../..");
const SHARED_CN_BACKEND_PORT = 3000;
const DEMO_APP_PRODUCT_NAME = "ctx-demo";
const DEMO_APP_IDENTIFIER = "rs.ctx.desktop.demo";

function repoPath(...segments) {
  return path.resolve(REPO_ROOT, ...segments);
}

const DESKTOP_PACKAGE = JSON.parse(readFileSync(repoPath("core/apps/desktop/package.json"), "utf8"));

function runStamp() {
  return new Date().toISOString().replace(/[-:.]/g, "").replace("T", "-").replace("Z", "");
}

function pickUnusedPortSync(fallback) {
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

function parseArgs(argv) {
  const stamp = runStamp();
  const baseArtifactDir = `/tmp/ctx-demo-playback-${stamp}`;
  const tauriTargetDir = `/tmp/ctx-demo-desktop-target-${stamp}`;
  const overrides = parseArgOverrides(argv);
  const options = {
    artifactDir: baseArtifactDir,
    workspaceRoot: `/tmp/ctx-demo-playback-workspace-${stamp}`,
    daemonDataDir: `/tmp/ctx-demo-daemon-ping-pong-${stamp}`,
    tauriTargetDir,
    appPath: path.join(tauriTargetDir, `debug/bundle/macos/${DEMO_APP_PRODUCT_NAME}.app`),
    promptText: "Make a ping pong game.",
    skipBuild: false,
    keepAlive: false,
    backendPort: pickUnusedPortSync(3000),
    driverPort: pickUnusedPortSync(4451),
    daemonPort: pickUnusedPortSync(4416),
    fixturePath: repoPath("core/apps/desktop/automation/fixtures/demo-ping-pong-fixture.json"),
    relayScenarioPath: repoPath("core/apps/desktop/automation/fixtures/demo-relay/codex-ping-pong.replay.json"),
    ...overrides,
  };
  if (!overrides.appPath) {
    options.appPath = path.join(options.tauriTargetDir, `debug/bundle/macos/${DEMO_APP_PRODUCT_NAME}.app`);
  }
  return options;
}

function parseArgOverrides(argv) {
  const out = {};
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const next = argv[index + 1];
    if (arg === "--artifact-dir") {
      out.artifactDir = path.resolve(next);
      index += 1;
    } else if (arg === "--workspace-root") {
      out.workspaceRoot = path.resolve(next);
      index += 1;
    } else if (arg === "--app-path") {
      out.appPath = path.resolve(next);
      index += 1;
    } else if (arg === "--daemon-data-dir") {
      out.daemonDataDir = path.resolve(next);
      index += 1;
    } else if (arg === "--tauri-target-dir") {
      out.tauriTargetDir = path.resolve(next);
      index += 1;
    } else if (arg === "--prompt") {
      out.promptText = next;
      index += 1;
    } else if (arg === "--skip-build") {
      out.skipBuild = true;
    } else if (arg === "--keep-alive") {
      out.keepAlive = true;
    } else if (arg === "--backend-port") {
      out.backendPort = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--driver-port") {
      out.driverPort = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--daemon-port") {
      out.daemonPort = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--fixture") {
      out.fixturePath = path.resolve(next);
      index += 1;
    } else if (arg === "--relay-scenario") {
      out.relayScenarioPath = path.resolve(next);
      index += 1;
    } else if (arg === "--help") {
      printHelp();
      process.exit(0);
    }
  }
  return out;
}

function printHelp() {
  process.stdout.write(`demo_ping_pong_playback

Usage:
  node core/apps/desktop/scripts/demo_ping_pong_playback.mjs

This command:
  1. seeds the ping pong fixture and replay relay
  2. builds/launches the automation-capable desktop app
  3. measures live DOM targets via CrabNebula WebDriver
  4. uses the native macOS conductor for visible prompt typing and diff clicks
  5. waits for the mocked Codex turn to finish and verifies the diff pane opens
`);
}

function ensureDir(dir) {
  mkdirSync(dir, { recursive: true });
  return dir;
}

function agentServerConfigPath(dataDir) {
  return path.join(dataDir, "providers", "agent-servers", "agent_servers.json");
}

function findBundledProviderRuntime(providerId, binaryName) {
  const root = path.join(os.homedir(), ".ctx", "providers", "agent-servers", providerId);
  if (!existsSync(root)) {
    return null;
  }
  const candidates = readdirSync(root, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => path.join(root, entry.name, binaryName))
    .filter((candidate) => existsSync(candidate))
    .sort()
    .reverse();
  return candidates[0] || null;
}

function seedProviderRuntimeConfig(dataDir, providerId, commandAbsPath) {
  const configPath = agentServerConfigPath(dataDir);
  const existing = existsSync(configPath)
    ? JSON.parse(readFileSync(configPath, "utf8"))
    : {
      providers: {},
      provider_login_commands: {},
      managed_installs: {},
      managed_provider_targets: {},
      managed_install_targets: {},
    };
  existing.providers = existing.providers || {};
  existing.providers[providerId] = {
    command: commandAbsPath,
    args: [],
    dependencies: [],
  };
  mkdirSync(path.dirname(configPath), { recursive: true });
  writeFileSync(configPath, `${JSON.stringify(existing, null, 2)}\n`, "utf8");
  return configPath;
}

function runChecked(command, args, options = {}) {
  const result = spawnSync(command, args, {
    stdio: "inherit",
    timeout: options.timeout ?? 0,
    ...options,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with status ${result.status}`);
  }
}

function desktopSyncCommand(profile = "debug") {
  return [process.execPath, ["core/scripts/desktop_sync_resources.cjs", "--profile", profile]];
}

function playbackBuildEnv(tauriTargetDir) {
  return {
    ...process.env,
    CARGO_TARGET_DIR: tauriTargetDir,
    CTX_DESKTOP_SYNC_BUNDLES: "0",
    CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD: "1",
  };
}

function resolveDaemonBinaryPath(tauriTargetDir = null) {
  void tauriTargetDir;
  return repoPath(
    "core/apps/desktop/src-tauri/bin",
    process.platform === "win32" ? "ctx-daemon.exe" : "ctx-daemon",
  );
}

function buildPlaybackAppEnv(setupManifest, workspaceRoot, authToken, tauriTargetDir = null) {
  const resolvedTargetDir = tauriTargetDir ? path.resolve(tauriTargetDir) : null;
  return {
    CTX_DESKTOP_DAEMON_URL: setupManifest.daemon.url,
    CTX_DESKTOP_DAEMON_TOKEN: authToken,
    CTX_DESKTOP_ALLOW_DEMO_COMMANDS: "1",
    CTX_DESKTOP_START_PATH: `/workspaces/${encodeURIComponent(setupManifest.fixture.workspace_id)}?ctxE2E=1`,
    CTX_AUTOMATION_WORKSPACE_PATH: workspaceRoot,
    ...(resolvedTargetDir
      ? {
        CTX_BUNDLE_DIR: path.join(resolvedTargetDir, "debug", "bundles"),
        CTX_DESKTOP_DEV_BIN_DIR: path.join(resolvedTargetDir, "debug"),
      }
      : {}),
  };
}

const TRUTHY_DETAIL_FLAGS = new Set(["1", "true", "yes", "on"]);

function providerDetailFlag(details, key) {
  const raw = details && typeof details === "object" ? details[key] : null;
  return TRUTHY_DETAIL_FLAGS.has(String(raw ?? "").trim().toLowerCase());
}

export function selectInstallableVisibleHarnessProviderIds(providerStatuses) {
  const statuses = Array.isArray(providerStatuses) ? providerStatuses : [];
  return statuses
    .filter((status) => status && typeof status.provider_id === "string")
    .filter((status) => status.details?.provider_kind !== "dependency")
    .filter((status) => !providerDetailFlag(status.details, "ui_hidden"))
    .filter((status) => providerDetailFlag(status.details, "install_supported"))
    .filter((status) => status.installed !== true)
    .map((status) => status.provider_id)
    .sort((left, right) => left.localeCompare(right));
}

export async function installVisibleHarnessProvidersAndWait(baseUrl, token, target = "host") {
  const query = `?target=${encodeURIComponent(target)}`;
  const before = await api(baseUrl, token, "GET", `/api/providers${query}`);
  const pendingBefore = selectInstallableVisibleHarnessProviderIds(before);
  if (pendingBefore.length === 0) {
    return {
      before,
      requested_provider_ids: [],
      after: before,
    };
  }

  const installStarts = await api(baseUrl, token, "POST", `/api/providers/install_all${query}`, {});
  const requestedProviderIds = Array.isArray(installStarts)
    ? installStarts
      .map((entry) => String(entry?.provider_id || "").trim())
      .filter((providerId) => providerId.length > 0)
    : [];

  const after = await waitFor(async () => {
    const statuses = await api(baseUrl, token, "GET", `/api/providers${query}`);
    return selectInstallableVisibleHarnessProviderIds(statuses).length === 0 ? statuses : null;
  }, {
    timeoutMs: 10 * 60_000,
    intervalMs: 2_000,
    label: `visible harness installs for target=${target}`,
  });

  return {
    before,
    requested_provider_ids: requestedProviderIds,
    after,
  };
}

function buildCnBackendEnv(appEnv, effectiveBackendPort) {
  return {
    ...process.env,
    ...appEnv,
    TEST_RUNNER_BACKEND_PORT: String(effectiveBackendPort),
  };
}

function buildCnDriverEnv(appEnv, driverPort, backendUrl) {
  return {
    ...process.env,
    ...appEnv,
    TAURI_DRIVER_PORT: String(driverPort),
    REMOTE_WEBDRIVER_URL: backendUrl,
  };
}

function buildDemoDesktopConnectionPayload(daemonUrl, authToken) {
  return {
    sessionConnection: JSON.stringify({
      v: 1,
      baseUrl: daemonUrl,
      wsBaseUrl: daemonUrl.replace(/^http/i, "ws"),
      authToken,
      source: "desktop",
    }),
    persistedBase: JSON.stringify({
      v: 1,
      baseUrl: daemonUrl,
      wsBaseUrl: daemonUrl.replace(/^http/i, "ws"),
    }),
  };
}

function buildDemoWorkbenchWindowPayload(workspaceId, taskId, sessionId, windowId = "ctx-demo-window") {
  const leafId = "ctx-demo-leaf";
  const tabId = "ctx-demo-task-tab";
  return {
    windowId,
    windowName: `ctx-ui-window-id:${windowId}`,
    sessionWindowKey: `wb.window.session.v1.${workspaceId}.${windowId}`,
    sessionWindow: JSON.stringify({
      v: 1,
      layout: {
        kind: "leaf",
        id: leafId,
        tabs: [
          {
            id: tabId,
            kind: "task",
            ref: {
              taskId,
              sessionId,
            },
          },
        ],
        activeTabId: tabId,
      },
      focusedLeafId: leafId,
      scrollByKey: {},
    }),
  };
}

function buildAutomationAppIfNeeded(appPath, skipBuild, tauriTargetDir) {
  if (skipBuild && existsSync(appPath)) {
    return;
  }
  const buildEnv = playbackBuildEnv(tauriTargetDir);
  if (!skipBuild) {
    runChecked("pnpm", ["-C", "core", "desktop:prep"], {
      cwd: REPO_ROOT,
      env: buildEnv,
    });
    const [syncCommand, syncArgs] = desktopSyncCommand("debug");
    runChecked(syncCommand, syncArgs, {
      cwd: REPO_ROOT,
      env: buildEnv,
    });
    runChecked(
      "pnpm",
      [
        "exec",
        "tauri",
        "build",
        "--debug",
        "--bundles",
        "app",
        "--config",
        JSON.stringify({
          productName: DEMO_APP_PRODUCT_NAME,
          identifier: DEMO_APP_IDENTIFIER,
        }),
        "--",
        "--features",
        "automation",
      ],
      {
        cwd: repoPath("core/apps/desktop/src-tauri"),
        env: buildEnv,
      },
    );
  }
  if (!existsSync(appPath)) {
    throw new Error(`automation app bundle not found: ${appPath}`);
  }
}

function prepareAutomationAppForLaunch(appPath, artifactDir) {
  if (process.platform !== "darwin") {
    return appPath;
  }
  const launchAppPath = path.join(artifactDir, `${DEMO_APP_PRODUCT_NAME}.app`);
  rmSync(launchAppPath, { recursive: true, force: true });
  cpSync(appPath, launchAppPath, { recursive: true, force: true });
  const infoPlistPath = path.join(launchAppPath, "Contents/Info.plist");
  for (const [key, value] of [
    ["CFBundleIdentifier", DEMO_APP_IDENTIFIER],
    ["CFBundleName", DEMO_APP_PRODUCT_NAME],
    ["CFBundleDisplayName", "ctx demo"],
    ["CFBundleExecutable", DEMO_APP_PRODUCT_NAME],
  ]) {
    const result = spawnSync("/usr/bin/plutil", ["-replace", key, "-string", value, infoPlistPath], {
      encoding: "utf8",
    });
    if (result.status !== 0) {
      throw new Error(
        `failed to rewrite ${key} in ${infoPlistPath}: ${String(result.stderr || result.stdout || "").trim()}`,
      );
    }
  }
  const originalExecutablePath = path.join(launchAppPath, "Contents/MacOS/ctx");
  const launchExecutablePath = path.join(launchAppPath, `Contents/MacOS/${DEMO_APP_PRODUCT_NAME}`);
  if (!existsSync(originalExecutablePath)) {
    throw new Error(`automation app executable not found: ${originalExecutablePath}`);
  }
  renameSync(originalExecutablePath, launchExecutablePath);
  return launchAppPath;
}

function startManagedProcess(command, args, options = {}) {
  const proc = spawn(command, args, {
    stdio: ["ignore", "pipe", "pipe"],
    ...options,
  });
  proc.stdout.on("data", () => {});
  proc.stderr.on("data", () => {});
  return proc;
}

function createCliAlias(targetCliPath, aliasName, artifactDir) {
  const aliasDir = mkdtempSync(path.join(artifactDir || os.tmpdir(), "ctx-cn-cli-"));
  const aliasPath = path.join(aliasDir, aliasName);
  symlinkSync(targetCliPath, aliasPath);
  return aliasPath;
}

function attachProcessDiagnostics(name, proc) {
  if (!proc) return;
  proc.on("error", (error) => {
    process.stderr.write(`[demo-playback] ${name} error: ${String(error?.stack || error)}\n`);
  });
  proc.on("exit", (code, signal) => {
    process.stderr.write(`[demo-playback] ${name} exited (code=${code ?? "null"}, signal=${signal ?? "null"})\n`);
  });
}

function waitForProcessReady({ proc, name, readyPromise, detail = "" }) {
  if (!proc) {
    throw new Error(`${name} process missing before readiness check`);
  }
  let onExit = null;
  const exitedBeforeReady = new Promise((_, reject) => {
    onExit = (code, signal) => {
      const suffix = detail ? ` ${detail}` : "";
      reject(new Error(`${name} exited before ready (code=${code ?? "null"}, signal=${signal ?? "null"}).${suffix}`));
    };
    proc.once("exit", onExit);
  });
  return Promise.race([readyPromise, exitedBeforeReady]).finally(() => {
    if (onExit) {
      proc.off("exit", onExit);
    }
  });
}

function waitForTcpPort(host, port, timeoutMs, label) {
  return new Promise((resolve, reject) => {
    const startedAt = Date.now();
    const tryConnect = () => {
      const socket = net.createConnection({ host, port });
      let settled = false;
      const finish = (error) => {
        if (settled) return;
        settled = true;
        socket.destroy();
        if (!error) {
          resolve();
          return;
        }
        if (Date.now() - startedAt >= timeoutMs) {
          reject(new Error(`${label} did not become ready within ${timeoutMs}ms: ${String(error.message || error)}`));
          return;
        }
        setTimeout(tryConnect, 200);
      };
      socket.setTimeout(1_000);
      socket.once("connect", () => finish(null));
      socket.once("timeout", () => finish(new Error("timeout")));
      socket.once("error", finish);
    };
    tryConnect();
  });
}

function bundleExecutablePaths(appPath) {
  const candidates = new Set([
    path.join(appPath, "Contents/MacOS/ctx"),
    path.join(appPath, `Contents/MacOS/${DEMO_APP_PRODUCT_NAME}`),
  ]);
  try {
    candidates.add(path.join(realpathSync(appPath), "Contents/MacOS/ctx"));
    candidates.add(path.join(realpathSync(appPath), `Contents/MacOS/${DEMO_APP_PRODUCT_NAME}`));
  } catch {
    // ignore missing/unresolvable bundle path here; buildAutomationAppIfNeeded validates existence
  }
  return [...candidates];
}

function killProcesses(matcher) {
  const out = spawnSync("ps", ["-Ao", "pid=,command="], { encoding: "utf8" });
  if (out.status !== 0) {
    return [];
  }
  const lines = String(out.stdout || "").split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
  const pids = [];
  for (const line of lines) {
    const match = line.match(/^(\d+)\s+(.*)$/);
    if (!match) continue;
    const pid = Number.parseInt(match[1], 10);
    const command = match[2] || "";
    if (!pid || !command || pid === process.pid) continue;
    if (matcher(pid, command)) {
      pids.push(pid);
    }
  }
  if (pids.length > 0) {
    spawnSync("kill", ["-9", ...pids.map(String)], { stdio: "ignore" });
  }
  return pids;
}

function matchingPidsForCommandFragment(fragment) {
  const result = spawnSync("pgrep", ["-f", fragment], {
    encoding: "utf8",
  });
  if (result.status !== 0 && result.status !== 1) {
    throw new Error(`pgrep failed for command fragment ${fragment}: ${String(result.stderr || result.stdout || "").trim()}`);
  }
  return String(result.stdout || "")
    .split(/\s+/)
    .map((value) => Number.parseInt(value, 10))
    .filter((value) => Number.isInteger(value) && value > 0 && value !== process.pid);
}

async function terminateAutomationAppProcesses(appPath) {
  const pids = [...new Set(bundleExecutablePaths(appPath).flatMap((fragment) => matchingPidsForCommandFragment(fragment)))];
  if (pids.length === 0) {
    return;
  }
  for (const pid of pids) {
    try {
      process.kill(pid, "SIGTERM");
    } catch (error) {
      if (error?.code !== "ESRCH") {
        throw error;
      }
    }
  }
  await waitFor(async () => {
    const stillRunning = pids.flatMap((pid) => {
      try {
        process.kill(pid, 0);
        return [pid];
      } catch (error) {
        if (error?.code === "ESRCH") {
          return [];
        }
        throw error;
      }
    });
    return stillRunning.length === 0 ? true : null;
  }, {
    timeoutMs: 15_000,
    intervalMs: 250,
    label: `automation app shutdown ${bundleExecutablePaths(appPath).join(", ")}`,
  });
}

function isTcpPortOpen(host, port) {
  return new Promise((resolve) => {
    const socket = net.createConnection({ host, port });
    let settled = false;
    const finish = (open) => {
      if (settled) return;
      settled = true;
      socket.destroy();
      resolve(open);
    };
    socket.setTimeout(500);
    socket.once("connect", () => finish(true));
    socket.once("timeout", () => finish(false));
    socket.once("error", () => finish(false));
  });
}

function listeningPidsForPort(port) {
  const result = spawnSync("lsof", ["-nP", `-iTCP:${port}`, "-sTCP:LISTEN", "-t"], {
    encoding: "utf8",
  });
  if (result.status !== 0 && result.status !== 1) {
    throw new Error(`lsof failed for port ${port}: ${String(result.stderr || result.stdout || "").trim()}`);
  }
  return String(result.stdout || "")
    .split(/\s+/)
    .map((value) => Number.parseInt(value, 10))
    .filter((value) => Number.isInteger(value) && value > 0);
}

function describeListeningProcessesForPort(port) {
  const pids = listeningPidsForPort(port);
  if (pids.length === 0) {
    return [];
  }
  const result = spawnSync("ps", ["-o", "pid=,command=", "-p", pids.join(",")], {
    encoding: "utf8",
  });
  if (result.status !== 0) {
    throw new Error(`ps failed for port ${port}: ${String(result.stderr || result.stdout || "").trim()}`);
  }
  return String(result.stdout || "")
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const match = line.match(/^(\d+)\s+(.+)$/);
      return match
        ? { pid: Number.parseInt(match[1], 10), command: match[2] }
        : null;
    })
    .filter((entry) => entry && Number.isInteger(entry.pid) && entry.pid > 0 && entry.command)
    .map((entry) => ({ pid: entry.pid, command: entry.command }));
}

function isCrabNebulaBackendCommand(command) {
  const text = String(command || "");
  return text.includes("ctx-cnb-cli")
    || text.includes("@crabnebula/test-runner-backend")
    || text.includes("test-runner-backend/cli.js");
}

function shouldRecycleSharedCrabNebulaBackend({
  platform = process.platform,
  backendPort,
  backendAlreadyListening,
  processEntries,
}) {
  if (!backendAlreadyListening) return false;
  if (platform !== "darwin") return false;
  if (backendPort !== SHARED_CN_BACKEND_PORT) return false;
  const entries = Array.isArray(processEntries) ? processEntries : [];
  if (entries.length === 0) return false;
  return entries.every((entry) => isCrabNebulaBackendCommand(entry.command));
}

async function terminateListeningPort(host, port, label) {
  const pids = listeningPidsForPort(port);
  if (pids.length === 0) {
    return;
  }
  for (const pid of pids) {
    try {
      process.kill(pid, "SIGTERM");
    } catch (error) {
      if (error?.code !== "ESRCH") {
        throw error;
      }
    }
  }
  await waitFor(async () => !(await isTcpPortOpen(host, port)), {
    timeoutMs: 15_000,
    intervalMs: 200,
    label: `${label} port ${port} shutdown`,
  });
}

function logFileStdio(logPath) {
  mkdirSync(path.dirname(logPath), { recursive: true });
  const fd = openSync(logPath, "a");
  return ["ignore", fd, fd];
}

function resolveBackendPort(requestedPort) {
  if (process.platform === "darwin") {
    return SHARED_CN_BACKEND_PORT;
  }
  return requestedPort;
}

function resolveWorkspaceModule(specifier) {
  try {
    return require.resolve(specifier, {
      paths: [
        repoPath("core"),
        repoPath("core/apps/desktop"),
        REPO_ROOT,
      ],
    });
  } catch (error) {
    const pnpmStoreRoot = repoPath("core/node_modules/.pnpm");
    const packageName = specifier.replace(/\/cli\.js$/, "");
    const packageDir = packageName.replace("/", "+");
    if (existsSync(pnpmStoreRoot)) {
      const match = readdirSync(pnpmStoreRoot, { withFileTypes: true })
        .filter((entry) => entry.isDirectory() && entry.name.startsWith(`${packageDir}@`))
        .map((entry) => path.join(pnpmStoreRoot, entry.name, "node_modules", packageName, "cli.js"))
        .find((candidate) => existsSync(candidate));
      if (match) {
        return match;
      }
    }
    throw error;
  }
}

function resolveDesktopDependencyVersion(packageName) {
  const version = DESKTOP_PACKAGE.devDependencies?.[packageName];
  if (typeof version !== "string" || version.trim().length === 0) {
    throw new Error(`missing desktop devDependency version for ${packageName}`);
  }
  return version;
}

function buildCrabNebulaCliCommand({ packageName, specifier, aliasName, cliArgs, artifactDir }) {
  try {
    const cliPath = resolveWorkspaceModule(specifier);
    const cliAlias = createCliAlias(cliPath, aliasName, artifactDir);
    return {
      command: process.execPath,
      args: [cliAlias, ...cliArgs],
      cwd: repoPath("core"),
    };
  } catch {
    return {
      command: "pnpm",
      args: ["-C", repoPath("core/apps/desktop"), "dlx", `${packageName}@${resolveDesktopDependencyVersion(packageName)}`, ...cliArgs],
      cwd: REPO_ROOT,
    };
  }
}

function requireWorkspacePackage(specifier, installCommand = "pnpm -C core install --frozen-lockfile") {
  try {
    const resolved = require.resolve(specifier, {
      paths: [
        repoPath("core"),
        repoPath("core/apps/desktop"),
        REPO_ROOT,
      ],
    });
    return require(resolved);
  } catch (error) {
    throw new Error(
      `missing workspace dependency "${specifier}". Run \`${installCommand}\` before running the demo playback.`,
      { cause: error },
    );
  }
}

async function startCrabNebulaStack({ artifactDir, backendPort, driverPort, appEnv }) {
  const backendHost = "127.0.0.1";
  const effectiveBackendPort = resolveBackendPort(backendPort);
  const backendLogPath = path.join(artifactDir, "test-runner-backend.log");
  const driverLogPath = path.join(artifactDir, "tauri-driver.log");
  let backendProc = null;
  const backendAlreadyListening = await isTcpPortOpen(backendHost, effectiveBackendPort);
  const backendProcessEntries = backendAlreadyListening
    ? describeListeningProcessesForPort(effectiveBackendPort)
    : [];
  const recycleSharedBackend = shouldRecycleSharedCrabNebulaBackend({
    backendPort: effectiveBackendPort,
    backendAlreadyListening,
    processEntries: backendProcessEntries,
  });
  if (
    backendAlreadyListening
    && process.platform === "darwin"
    && effectiveBackendPort === SHARED_CN_BACKEND_PORT
    && !recycleSharedBackend
  ) {
    const summary = backendProcessEntries.length > 0
      ? backendProcessEntries.map(({ pid, command }) => `${pid}:${command}`).join("; ")
      : "unknown listener";
    throw new Error(
      `shared CrabNebula backend port ${effectiveBackendPort} is occupied by a non-playback process: ${summary}`,
    );
  }
  if (recycleSharedBackend) {
    await terminateListeningPort(backendHost, effectiveBackendPort, "CrabNebula backend");
  }
  if (!backendAlreadyListening || recycleSharedBackend) {
    const backendCommand = buildCrabNebulaCliCommand({
      packageName: "@crabnebula/test-runner-backend",
      specifier: "@crabnebula/test-runner-backend/cli.js",
      aliasName: "ctx-cnb-cli",
      cliArgs: ["--host", backendHost, "--port", String(effectiveBackendPort)],
      artifactDir,
    });
    backendProc = spawn(backendCommand.command, backendCommand.args, {
      cwd: backendCommand.cwd,
      env: buildCnBackendEnv(appEnv, effectiveBackendPort),
      stdio: logFileStdio(backendLogPath),
    });
    attachProcessDiagnostics("test-runner-backend", backendProc);
    await waitForProcessReady({
      proc: backendProc,
      name: "test-runner-backend",
      readyPromise: waitForTcpPort(backendHost, effectiveBackendPort, 30_000, "CrabNebula backend"),
      detail: `Requested backend port=${String(backendPort)} effective=${String(effectiveBackendPort)}.`,
    });
  }

  const driverCommand = buildCrabNebulaCliCommand({
    packageName: "@crabnebula/tauri-driver",
    specifier: "@crabnebula/tauri-driver/cli.js",
    aliasName: "ctx-tdrv-cli",
    cliArgs: ["--port", String(driverPort)],
    artifactDir,
  });
  const driverProc = spawn(driverCommand.command, driverCommand.args, {
    cwd: driverCommand.cwd,
    env: buildCnDriverEnv(appEnv, driverPort, `http://${backendHost}:${effectiveBackendPort}`),
    stdio: logFileStdio(driverLogPath),
  });
  attachProcessDiagnostics("tauri-driver", driverProc);
  await waitForProcessReady({
    proc: driverProc,
    name: "tauri-driver",
    readyPromise: waitForTcpPort("127.0.0.1", driverPort, 30_000, "tauri-driver"),
    detail: `Requested driver port=${String(driverPort)} backend=http://${backendHost}:${effectiveBackendPort}.`,
  });

  return { backendProc, driverProc, effectiveBackendPort, backendLogPath, driverLogPath };
}

function stopManagedProcess(proc) {
  if (!proc || proc.killed) return;
  proc.kill("SIGTERM");
}

async function connectBrowser({ driverPort, appPath }) {
  const { remote } = requireWorkspacePackage("webdriverio");
  return remote({
    hostname: "127.0.0.1",
    port: driverPort,
    path: "/",
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

async function waitForSelector(browser, selector, timeoutMs = 60_000) {
  await browser.waitUntil(
    async () => await browser.execute((currentSelector) => Boolean(document.querySelector(currentSelector)), selector),
    { timeout: timeoutMs, timeoutMsg: `selector not found: ${selector}` },
  );
}

async function captureBrowserState(browser) {
  return browser.execute(() => {
    const normalize = (value, limit = 2_000) => {
      const text = typeof value === "string" ? value : String(value || "");
      return text.length > limit ? `${text.slice(0, limit)}…` : text;
    };
    return {
      href: window.location.href,
      pathname: window.location.pathname,
      search: window.location.search,
      title: document.title,
      readyState: document.readyState,
      bodyClassName: document.body?.className || "",
      rootSnippet: normalize(document.getElementById("root")?.innerHTML || "", 8_000),
      bodySnippet: normalize(document.body?.innerHTML || "", 8_000),
    };
  });
}

async function navigateBrowserToWorkspace(browser, workspaceId) {
  const workspacePath = `/workspaces/${encodeURIComponent(workspaceId)}`;
  await browser.execute((path) => {
    window.location.href = path;
  }, workspacePath);
  await browser.waitUntil(
    async () => {
      const state = await captureBrowserState(browser);
      return state.pathname === workspacePath;
    },
    {
      timeout: 30_000,
      timeoutMsg: `browser did not navigate to ${workspacePath}`,
    },
  );
}

async function primeDemoDesktopConnectionToWorkspace(browser, daemonUrl, authToken, workspaceId) {
  const storagePayload = buildDemoDesktopConnectionPayload(daemonUrl, authToken);
  await browser.execute(
    async ({ nextBaseUrl, nextToken, storage }) => {
      const invoke = globalThis.__TAURI__?.core?.invoke || globalThis.__TAURI_INTERNALS__?.invoke;
      if (typeof invoke !== "function") {
        throw new Error("Tauri invoke bridge unavailable in automation app");
      }
      await invoke("desktop_set_demo_connection", {
        req: {
          base_url: nextBaseUrl,
          token: nextToken,
        },
      });
      sessionStorage.setItem("ctxE2E", "1");
      sessionStorage.setItem("ctxDaemonConnectionV1", storage.sessionConnection);
      localStorage.setItem("ctxDaemonConnectionBaseV1", storage.persistedBase);
    },
    {
      nextBaseUrl: daemonUrl,
      nextToken: authToken,
      storage: storagePayload,
    },
  );
  await navigateBrowserToWorkspace(browser, workspaceId);
}

async function primeDemoDesktopConnection(browser, daemonUrl, authToken, workspaceId, taskId, sessionId) {
  const workspacePath = `/workspaces/${encodeURIComponent(workspaceId)}?ctxE2E=1`;
  await primeDemoDesktopConnectionToWorkspace(browser, daemonUrl, authToken, workspaceId);
  const workbenchPayload = buildDemoWorkbenchWindowPayload(workspaceId, taskId, sessionId);
  await browser.execute(
    (workbench) => {
      sessionStorage.setItem("contextUiWindowId.v1", workbench.windowId);
      sessionStorage.setItem(workbench.sessionWindowKey, workbench.sessionWindow);
      if (typeof window.name === "string" && (!window.name || window.name.startsWith("ctx-ui-window-id:"))) {
        window.name = workbench.windowName;
      }
    },
    workbenchPayload,
  );
  await browser.waitUntil(
    async () =>
      await browser.execute(() => Boolean(globalThis.__ctxE2E && typeof globalThis.__ctxE2E.focusTask === "function")),
    {
      timeout: 30_000,
      timeoutMsg: "ctxE2E focusTask bridge did not become available",
    },
  );
  await browser.waitUntil(
    async () =>
      await browser.execute(
        (payload) => {
          const focusTask = globalThis.__ctxE2E?.focusTask;
          if (typeof focusTask !== "function") {
            return false;
          }
          return focusTask(payload.taskId, payload.sessionId);
        },
        {
          taskId: taskId,
          sessionId: sessionId,
        },
      ),
    {
      timeout: 30_000,
      timeoutMsg: `failed to focus task ${taskId}`,
    },
  );
  await browser.waitUntil(
    async () => {
      const state = await captureBrowserState(browser);
      return state.pathname === `/workspaces/${encodeURIComponent(workspaceId)}`;
    },
    {
      timeout: 30_000,
      timeoutMsg: `browser did not navigate to ${workspacePath} after demo connection priming`,
    },
  );
}

async function measureTargets(browser, selectors) {
  return browser.execute(async (requestedSelectors) => {
    const bridge = globalThis.__ctxE2E;
    if (!bridge || typeof bridge.measureTargets !== "function") {
      throw new Error("ctxE2E measureTargets bridge unavailable");
    }
    return bridge.measureTargets(requestedSelectors);
  }, selectors);
}

async function waitForSessionTurnCompletion(baseUrl, token, sessionId, options = {}) {
  const apiImpl = options.apiImpl || api;
  return waitFor(async () => {
    const response = await apiImpl(baseUrl, token, "GET", `/api/sessions/${sessionId}/events?limit=400`);
    const events = Array.isArray(response) ? response : Array.isArray(response?.events) ? response.events : [];
    const turnFinished = events.find(
      (event) =>
        Number(event.seq || 0) > Number(options.minSeqExclusive || 0) &&
        String(event.event_type) === "turn_finished" &&
        String(event.payload_json?.status || "").toLowerCase() === "completed",
    );
    return turnFinished || null;
  }, {
    timeoutMs: options.timeoutMs ?? 180_000,
    intervalMs: options.intervalMs ?? 1_000,
    label: `session ${sessionId} turn completion`,
  });
}

async function readSessionEventSeq(baseUrl, token, sessionId, options = {}) {
  const apiImpl = options.apiImpl || api;
  const response = await apiImpl(baseUrl, token, "GET", `/api/sessions/${sessionId}/events?limit=400`);
  const events = Array.isArray(response) ? response : Array.isArray(response?.events) ? response.events : [];
  return events.reduce((maxSeq, event) => {
    const seq = Number(event?.seq || 0);
    return seq > maxSeq ? seq : maxSeq;
  }, 0);
}

async function submitComposerPrompt(browser, promptText) {
  return browser.execute((text) => {
    const textarea = document.querySelector(
      ".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea, textarea.wb-new-composer-textarea",
    );
    if (!(textarea instanceof HTMLTextAreaElement)) {
      throw new Error("composer textarea not found");
    }
    const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value")?.set;
    if (typeof setter !== "function") {
      throw new Error("textarea value setter unavailable");
    }
    setter.call(textarea, text);
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
    textarea.dispatchEvent(new Event("change", { bubbles: true }));
    const sendButton = document.querySelector("button.wb-send[aria-label=\"Send\"]");
    if (!(sendButton instanceof HTMLButtonElement)) {
      throw new Error("send button not found");
    }
    if (sendButton.disabled) {
      throw new Error("send button is disabled after prompt sync");
    }
    sendButton.click();
    return {
      value: textarea.value,
      send_disabled: sendButton.disabled,
    };
  }, promptText);
}

function runConductor(scenarioPath) {
  process.stderr.write(`[demo-playback] running conductor for ${scenarioPath}\n`);
  return new Promise((resolve, reject) => {
    const proc = spawn("swift", [repoPath("core/apps/desktop/scripts/macos_demo_conductor.swift"), "--scenario", scenarioPath], {
      cwd: REPO_ROOT,
      env: { ...process.env },
      stdio: "inherit",
    });
    proc.once("error", reject);
    proc.once("exit", (status, signal) => {
      if (status === 0) {
        process.stderr.write(`[demo-playback] conductor completed for ${scenarioPath}\n`);
        resolve();
        return;
      }
      reject(new Error(
        signal
          ? `swift conductor exited from signal ${signal}`
          : `swift conductor exited with status ${status}`,
      ));
    });
  });
}

function writeScenario(pathname, scenario) {
  writeFileSync(pathname, `${JSON.stringify(scenario, null, 2)}\n`, "utf8");
}

function writeJsonArtifact(artifactPath, value) {
  writeFileSync(artifactPath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

function appendPlaybackStep(artifactDir, step, detail = {}) {
  const event = {
    ts: new Date().toISOString(),
    step,
    ...detail,
  };
  const logPath = path.join(artifactDir, "playback-steps.jsonl");
  writeFileSync(logPath, `${JSON.stringify(event)}\n`, { encoding: "utf8", flag: "a" });
  return event;
}

async function captureBrowserArtifact(browser, artifactDir, name) {
  const state = await captureBrowserState(browser).catch((error) => ({
    capture_error: String(error?.stack || error),
  }));
  writeJsonArtifact(path.join(artifactDir, name), state);
  return state;
}

async function startSetupProcess(options) {
  const relay = await startDemoRelay({
    mode: "replay",
    listenHost: "127.0.0.1",
    listenPort: 0,
    artifactDir: options.artifactDir,
    scenarioPath: options.relayScenarioPath,
    upstreamBaseUrl: "",
    upstreamApiKey: "",
    rewriteFrom: "",
    rewriteTo: "",
  });

  const daemonDataDir = options.daemonDataDir;
  const bundledCodexRuntime = findBundledProviderRuntime("codex", "codex-crp");
  if (bundledCodexRuntime) {
    seedProviderRuntimeConfig(daemonDataDir, "codex", bundledCodexRuntime);
  }

  const daemon = await startDaemon(daemonDataDir, `127.0.0.1:${options.daemonPort}`, {
    tauriTargetDir: options.tauriTargetDir,
    skipBuild: options.skipBuild,
  });
  const auth = readDaemonAuth({
    daemonUrl: daemon.daemonUrl,
    dataDir: daemon.dataDir,
  });

  const visibleHarnessInstalls = options.installVisibleHarnesses
    ? await installVisibleHarnessProvidersAndWait(auth.daemonUrl, auth.authToken, "host")
    : null;
  const providerStatusBefore = await getProviderStatus(auth.daemonUrl, auth.authToken, "codex", "host");
  const install = providerStatusBefore.installed
    ? null
    : await installProviderAndWait(auth.daemonUrl, auth.authToken, "codex", { target: "host" });
  const providerStatusAfterInstall = await getProviderStatus(auth.daemonUrl, auth.authToken, "codex", "host");

  const fixture = await bootstrapDemoFixture({
    scenarioPath: options.fixturePath,
    workspaceRoot: options.workspaceRoot,
    daemonUrl: auth.daemonUrl,
    authToken: auth.authToken,
    dataDir: daemon.dataDir,
  });

  const provider = await configureProviderEndpoint({
    providerId: "codex",
    endpointName: `demo-relay-${runStamp()}`,
    endpointId: `demo-relay-${runStamp()}`,
    baseUrl: `http://127.0.0.1:${relay.port}/v1`,
    modelId: "gpt-5.4",
    apiShape: "openai_responses",
    authType: "bearer",
    apiKey: "demo-key",
    daemonUrl: auth.daemonUrl,
    authToken: auth.authToken,
    dataDir: daemon.dataDir,
  });

  const manifest = {
    fixture,
    visible_harness_installs: visibleHarnessInstalls,
    provider_status_before: providerStatusBefore,
    install,
    provider_status_after_install: providerStatusAfterInstall,
    provider,
    verify: null,
    relay: {
      artifact_dir: options.artifactDir,
      scenario_path: options.relayScenarioPath,
      sse_path: null,
      port: relay.port,
    },
    daemon: {
      url: auth.daemonUrl,
      data_dir: daemon.dataDir,
      log_path: daemon.logPath,
    },
  };

  writeFileSync(path.join(options.artifactDir, "demo-manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  return { relay, daemon, manifest };
}

async function startDaemon(dataDir, bind, { tauriTargetDir = null, skipBuild = false } = {}) {
  mkdirSync(dataDir, { recursive: true });
  const [host, port] = bind.split(":");
  const logPath = path.join(dataDir, "daemon.log");
  const logStream = createWriteStream(logPath, { flags: "a" });
  const daemonBin = resolveDaemonBinaryPath(tauriTargetDir);
  if (!skipBuild || !existsSync(daemonBin)) {
    runChecked("cargo", ["build", "-p", "ctx-http", "--bin", "ctx"], {
      cwd: repoPath("core"),
      env: {
        ...process.env,
        CTX_DEV_MODE: "1",
        ...(tauriTargetDir ? { CARGO_TARGET_DIR: path.resolve(tauriTargetDir) } : {}),
      },
    });
    const [syncCommand, syncArgs] = desktopSyncCommand("debug");
    runChecked(syncCommand, syncArgs, {
      cwd: REPO_ROOT,
      env: playbackBuildEnv(tauriTargetDir ? path.resolve(tauriTargetDir) : repoPath("core/target")),
    });
  }
  const proc = spawn(
    daemonBin,
    ["serve", "--bind", `${host}:${port}`, "--data-dir", dataDir],
    {
      cwd: REPO_ROOT,
      env: {
        ...process.env,
        CTX_DEV_MODE: "1",
      },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  proc.stdout.pipe(logStream, { end: false });
  proc.stderr.pipe(logStream, { end: false });
  await waitForFile(path.join(dataDir, "daemon_auth.json"), { timeoutMs: 300_000, label: "daemon_auth.json" });
  await waitForHttpOk(`http://${host}:${port}/api/health`, { timeoutMs: 300_000, label: "daemon health" });
  return {
    proc,
    logPath,
    dataDir,
    daemonUrl: `http://${host}:${port}`,
  };
}

async function ensureWorkbenchVisible(browser, artifactDir) {
  try {
    await waitForSelector(browser, ".wb-root");
    await browser.waitUntil(
      async () =>
        await browser.execute(() => {
          const hasActiveSessionComposer = Boolean(
            document.querySelector(".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea"),
          );
          const hasWorkbenchShell = Boolean(document.querySelector(".wb-main"));
          const bridge = globalThis.__ctxE2E;
          const hasWorkbenchBridge = Boolean(
            bridge &&
            typeof bridge.focusNewTask === "function" &&
            typeof bridge.clearDraftHarness === "function" &&
            typeof bridge.toggleDiffPane === "function" &&
            typeof bridge.toggleArtifactsPane === "function" &&
            typeof bridge.measureTargets === "function" &&
            typeof bridge.measureHarnessOption === "function" &&
            typeof bridge.measureDiffFile === "function",
          );
          return hasActiveSessionComposer || (hasWorkbenchShell && hasWorkbenchBridge);
        }),
      {
        timeout: 60_000,
        timeoutMsg: "workbench interactive shell did not become visible",
      },
    );
  } catch (error) {
    const state = await captureBrowserState(browser).catch((captureError) => ({
      capture_error: String(captureError?.stack || captureError),
    }));
    writeFileSync(path.join(artifactDir, "browser-state-on-workbench-failure.json"), `${JSON.stringify(state, null, 2)}\n`, "utf8");
    throw error;
  }
}

async function waitForDiffPane(browser) {
  await browser.waitUntil(async () => {
    const state = await captureDiffPaneState(browser);
    return state.hasDiffPaneClass || state.hasDiffContent;
  }, { timeout: 30_000, timeoutMsg: "diff pane did not open in time" });
}

async function waitForDiffPaneReady(browser, options = {}) {
  const minFileRows = Math.max(1, Number(options.minFileRows ?? 1));
  const requireParsedSummaries = options.requireParsedSummaries !== false;
  const requiredStablePolls = Math.max(1, Number(options.requiredStablePolls ?? 2));
  let stablePolls = 0;
  let lastSignature = null;
  await browser.waitUntil(async () => {
    const state = await captureDiffPaneState(browser);
    const inventoryReady =
      state.hasDiffPaneClass &&
      state.fileRowCount >= minFileRows &&
      !state.loadingChangesVisible &&
      !state.loadingChangedFilesVisible;
    const parsedSummariesReady =
      !requireParsedSummaries ||
      (state.diffSummaryCount >= state.fileRowCount && state.statusSummaryCount === 0);
    if (!inventoryReady || !parsedSummariesReady) {
      stablePolls = 0;
      lastSignature = null;
      return false;
    }
    const signature = [
      state.fileRowCount,
      state.diffSummaryCount,
      state.statusSummaryCount,
      state.openFileCount,
      state.editorShellCount,
      state.monacoEditorCount,
    ].join("|");
    if (signature === lastSignature) {
      stablePolls += 1;
    } else {
      lastSignature = signature;
      stablePolls = 1;
    }
    return stablePolls >= requiredStablePolls;
  }, {
    timeout: 30_000,
    interval: 150,
    timeoutMsg: "diff pane did not reach a stable ready state in time",
  });
}

async function openDiffPane(browser) {
  await browser.waitUntil(
    async () =>
      await browser.execute(() => Boolean(globalThis.__ctxE2E && typeof globalThis.__ctxE2E.toggleDiffPane === "function")),
    {
      timeout: 30_000,
      timeoutMsg: "ctxE2E toggleDiffPane bridge did not become available",
    },
  );
  return browser.execute(() => {
    const toggleDiffPane = globalThis.__ctxE2E?.toggleDiffPane;
    if (typeof toggleDiffPane !== "function") {
      throw new Error("ctxE2E toggleDiffPane bridge unavailable");
    }
    const before = {
      diff_open: Boolean(document.querySelector(".wb-right-pane.wb-diff")),
      right_pane_count: document.querySelectorAll(".wb-right-pane").length,
    };
    const invoked = toggleDiffPane();
    const after = {
      diff_open: Boolean(document.querySelector(".wb-right-pane.wb-diff")),
      right_pane_count: document.querySelectorAll(".wb-right-pane").length,
    };
    return { invoked, before, after };
  });
}

async function captureDiffPaneState(browser) {
  return browser.execute(() => {
    const diffPane = document.querySelector(".wb-right-pane.wb-diff");
    const diffContent = document.querySelector(".wb-right-pane.wb-diff .diff-pane, .wb-right-pane.wb-diff .wb-diff-empty");
    const toggleButton = document.querySelector('button[aria-label="Toggle diff view"]');
    const mutedLabels = Array.from(document.querySelectorAll(".wb-right-pane.wb-diff .muted"))
      .map((element) => element.textContent?.trim() ?? "")
      .filter(Boolean);
    const fileRows = Array.from(document.querySelectorAll(".wb-right-pane.wb-diff .cursor-diff-file"));
    return {
      hasDiffPaneClass: Boolean(diffPane),
      hasDiffContent: Boolean(diffContent),
      rightPaneCount: document.querySelectorAll(".wb-right-pane").length,
      togglePressed: toggleButton instanceof HTMLButtonElement ? toggleButton.getAttribute("aria-pressed") === "true" : false,
      loadingChangesVisible: mutedLabels.includes("Loading changes..."),
      loadingChangedFilesVisible: mutedLabels.includes("Loading changed files..."),
      loadingDiffVisible: mutedLabels.includes("Loading diff..."),
      parsingDiffVisible: mutedLabels.includes("Parsing diff..."),
      fileRowCount: fileRows.length,
      openFileCount: fileRows.filter((row) => row.querySelector(".cursor-diff-file-body")).length,
      diffSummaryCount: document.querySelectorAll(
        '.wb-right-pane.wb-diff .cursor-diff-summary[aria-label="Diff summary"]',
      ).length,
      statusSummaryCount: document.querySelectorAll(
        '.wb-right-pane.wb-diff .cursor-diff-summary[aria-label="File status"]',
      ).length,
      editorShellCount: document.querySelectorAll(".wb-right-pane.wb-diff .cursor-diff-editor-shell").length,
      monacoEditorCount: document.querySelectorAll(".wb-right-pane.wb-diff .monaco-editor").length,
      hiddenByPlayback: Boolean(document.getElementById("ctx-demo-hide-diff-pane-style")),
    };
  });
}

async function setDiffPanePlaybackHidden(browser, hidden) {
  return browser.execute((nextHidden) => {
    const styleId = "ctx-demo-hide-diff-pane-style";
    const existing = document.getElementById(styleId);
    if (nextHidden) {
      if (!existing) {
        const style = document.createElement("style");
        style.id = styleId;
        style.textContent = ".wb-right-pane.wb-diff { display: none !important; }";
        document.head.appendChild(style);
      }
    } else {
      existing?.remove();
    }
    return Boolean(document.getElementById(styleId));
  }, Boolean(hidden));
}

async function setArtifactsPanePlaybackHidden(browser, hidden) {
  return browser.execute((nextHidden) => {
    const marker = "data-ctx-demo-artifacts-hidden";
    const panes = Array.from(document.querySelectorAll(".wb-right-pane")).filter(
      (element) => element.querySelector(".wb-artifacts"),
    );
    for (const pane of panes) {
      if (!(pane instanceof HTMLElement)) continue;
      if (nextHidden) {
        pane.setAttribute(marker, "1");
        pane.style.display = "none";
      } else if (pane.getAttribute(marker) === "1") {
        pane.removeAttribute(marker);
        pane.style.removeProperty("display");
      }
    }
    return panes.some((pane) => pane instanceof HTMLElement && pane.getAttribute(marker) === "1");
  }, Boolean(hidden));
}

async function waitForArtifactsPane(browser) {
  await browser.waitUntil(async () => {
    const state = await captureArtifactsPaneState(browser);
    return state.hasArtifactCard || state.hasVideoPreview || state.hasEmptyState;
  }, { timeout: 30_000, timeoutMsg: "artifacts pane did not open in time" });
}

async function openArtifactsPane(browser) {
  await browser.waitUntil(
    async () =>
      await browser.execute(
        () => Boolean(globalThis.__ctxE2E && typeof globalThis.__ctxE2E.toggleArtifactsPane === "function"),
      ),
    {
      timeout: 30_000,
      timeoutMsg: "ctxE2E toggleArtifactsPane bridge did not become available",
    },
  );
  return browser.execute(() => {
    const toggleArtifactsPane = globalThis.__ctxE2E?.toggleArtifactsPane;
    if (typeof toggleArtifactsPane !== "function") {
      throw new Error("ctxE2E toggleArtifactsPane bridge unavailable");
    }
    const before = {
      artifact_card_count: document.querySelectorAll(".wb-artifact-card").length,
      video_count: document.querySelectorAll(".wb-artifact-video").length,
    };
    const invoked = toggleArtifactsPane();
    const after = {
      artifact_card_count: document.querySelectorAll(".wb-artifact-card").length,
      video_count: document.querySelectorAll(".wb-artifact-video").length,
    };
    return { invoked, before, after };
  });
}

async function captureArtifactsPaneState(browser) {
  return browser.execute(() => {
    const artifactCards = document.querySelectorAll(".wb-artifact-card");
    const videos = Array.from(document.querySelectorAll(".wb-artifact-video")).filter(
      (element) => element instanceof HTMLVideoElement,
    );
    const empty = document.querySelector(".wb-artifact-empty, .wb-artifact-inline-status, .wb-artifact-missing");
    const playingVideos = videos.filter((element) => !element.paused && !element.ended);
    return {
      hasArtifactCard: artifactCards.length > 0,
      artifactCardCount: artifactCards.length,
      hasVideoPreview: videos.length > 0,
      videoCount: videos.length,
      playingVideoCount: playingVideos.length,
      firstVideoCurrentTime: videos[0]?.currentTime ?? 0,
      firstVideoDuration: Number.isFinite(videos[0]?.duration) ? videos[0].duration : null,
      firstVideoPaused: videos[0]?.paused ?? true,
      firstVideoEnded: videos[0]?.ended ?? false,
      firstVideoControls: videos[0]?.controls ?? false,
      firstVideoLoop: videos[0]?.loop ?? false,
      hasEmptyState: Boolean(empty),
      rightPaneCount: document.querySelectorAll(".wb-right-pane").length,
      hiddenByPlayback: Array.from(document.querySelectorAll(".wb-right-pane")).some(
        (element) => element instanceof HTMLElement && element.getAttribute("data-ctx-demo-artifacts-hidden") === "1",
      ),
    };
  });
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (process.platform !== "darwin") {
    throw new Error("demo_ping_pong_playback is currently macOS-only");
  }
  if (!String(process.env.CN_API_KEY || "").trim()) {
    throw new Error("CN_API_KEY is required on macOS; load it from Infisical before running this demo playback.");
  }
  ensureDir(options.artifactDir);
  ensureDir(options.workspaceRoot);

  let browser = null;
  let backendProc = null;
  let driverProc = null;
  let relayServer = null;
  let daemonProc = null;
  let verifyPromise = null;

  try {
    const setup = await startSetupProcess(options);
    relayServer = setup.relay.server;
    daemonProc = setup.daemon.proc;
    const setupManifest = setup.manifest;
    const appEnv = buildPlaybackAppEnv(
      setupManifest,
      options.workspaceRoot,
      readTokenFromManifest(setupManifest),
      options.tauriTargetDir,
    );
    verifyPromise = api(
      setupManifest.daemon.url,
      readTokenFromManifest(setupManifest),
      "POST",
      `/api/workspaces/${setupManifest.fixture.workspace_id}/providers/codex/verify`,
      {},
    ).catch((error) => {
      throw new Error(`codex provider verify failed: ${String(error?.stack || error)}`);
    });

    buildAutomationAppIfNeeded(options.appPath, options.skipBuild, options.tauriTargetDir);
    const launchAppPath = prepareAutomationAppForLaunch(options.appPath, options.artifactDir);
    await terminateAutomationAppProcesses(launchAppPath);

    const cn = await startCrabNebulaStack({
      artifactDir: options.artifactDir,
      backendPort: options.backendPort,
      driverPort: options.driverPort,
      appEnv,
    });
    backendProc = cn.backendProc;
    driverProc = cn.driverProc;
    browser = await connectBrowser({ driverPort: options.driverPort, appPath: launchAppPath });

    await primeDemoDesktopConnection(
      browser,
      setupManifest.daemon.url,
      readTokenFromManifest(setupManifest),
      setupManifest.fixture.workspace_id,
      setupManifest.fixture.task_id,
      setupManifest.fixture.session_id,
    );
    await ensureWorkbenchVisible(browser, options.artifactDir);

    const promptMeasurement = await measureTargets(browser, {
      composer: ".wb-session-slot[aria-hidden=\"false\"] textarea.wb-active-textarea",
    });
    if (!promptMeasurement.elements.composer) {
      throw new Error("composer target not found");
    }
    const composerPoint = screenPointFromProbe(promptMeasurement.metrics, promptMeasurement.elements.composer.rect);
    const promptScenarioPath = path.join(options.artifactDir, "prompt-scenario.json");
    writeScenario(promptScenarioPath, buildPromptScenario(composerPoint, options.promptText));
    writeJsonArtifact(path.join(options.artifactDir, "prompt-measurement.json"), promptMeasurement);
    appendPlaybackStep(options.artifactDir, "conductor_prompt_start");
    await runConductor(promptScenarioPath);
    appendPlaybackStep(options.artifactDir, "conductor_prompt_complete");
    const sessionEventBaseline = await readSessionEventSeq(
      setupManifest.daemon.url,
      readTokenFromManifest(setupManifest),
      setupManifest.fixture.session_id,
    );
    appendPlaybackStep(options.artifactDir, "session_event_baseline", {
      seq: sessionEventBaseline,
    });
    await submitComposerPrompt(browser, options.promptText);
    appendPlaybackStep(options.artifactDir, "composer_submitted");
    await captureBrowserArtifact(browser, options.artifactDir, "browser-state-after-send.json");

    appendPlaybackStep(options.artifactDir, "session_turn_wait_start");
    await waitForSessionTurnCompletion(
      setupManifest.daemon.url,
      readTokenFromManifest(setupManifest),
      setupManifest.fixture.session_id,
      { minSeqExclusive: sessionEventBaseline },
    );
    appendPlaybackStep(options.artifactDir, "session_turn_wait_complete");
    await sleep(1200);
    await captureBrowserArtifact(browser, options.artifactDir, "browser-state-before-diff.json");

    appendPlaybackStep(options.artifactDir, "diff_toggle_start");
    const diffToggleResult = await openDiffPane(browser);
    writeJsonArtifact(path.join(options.artifactDir, "diff-toggle-result.json"), diffToggleResult);
    appendPlaybackStep(options.artifactDir, "diff_toggle_complete", diffToggleResult);
    await captureBrowserArtifact(browser, options.artifactDir, "browser-state-after-diff-toggle.json");

    appendPlaybackStep(options.artifactDir, "diff_pane_wait_start");
    try {
      await waitForDiffPane(browser);
    } catch (error) {
      appendPlaybackStep(options.artifactDir, "diff_pane_wait_failed", {
        error: String(error?.stack || error),
      });
      await captureBrowserArtifact(browser, options.artifactDir, "browser-state-on-diff-failure.json");
      throw error;
    }
    appendPlaybackStep(options.artifactDir, "diff_pane_wait_complete");
    writeJsonArtifact(path.join(options.artifactDir, "diff-pane-state.json"), await captureDiffPaneState(browser));
    await captureBrowserArtifact(browser, options.artifactDir, "browser-state-after-diff.json");

    const result = {
      artifact_dir: options.artifactDir,
      workspace_root: options.workspaceRoot,
      session_id: setupManifest.fixture.session_id,
      daemon_url: setupManifest.daemon.url,
      relay_port: setupManifest.relay.port,
      app_path: options.appPath,
      prompt_scenario_path: promptScenarioPath,
      manifest: setupManifest,
      status: "ok",
    };
    writeFileSync(path.join(options.artifactDir, "playback-result.json"), `${JSON.stringify(result, null, 2)}\n`, "utf8");
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);

    if (options.keepAlive) {
      await new Promise(() => {});
    }
  } finally {
    if (browser) {
      try {
        await browser.deleteSession();
      } catch {
        // ignore
      }
    }
    stopManagedProcess(driverProc);
    stopManagedProcess(backendProc);
    if (relayServer) {
      try {
        relayServer.close();
      } catch {
        // ignore
      }
    }
    stopManagedProcess(daemonProc);
  }
}

function readTokenFromManifest(setup) {
  const authPath = path.join(setup.daemon.data_dir, "daemon_auth.json");
  const auth = JSON.parse(readFileSync(authPath, "utf8"));
  return String(auth.token || "").trim();
}

export {
  bundleExecutablePaths,
  buildAutomationAppIfNeeded,
  buildCnBackendEnv,
  buildCnDriverEnv,
  buildDemoDesktopConnectionPayload,
  buildDemoWorkbenchWindowPayload,
  buildPlaybackAppEnv,
  connectBrowser,
  captureBrowserState,
  createCliAlias,
  readTokenFromManifest,
  desktopSyncCommand,
  ensureWorkbenchVisible,
  findBundledProviderRuntime,
  isTcpPortOpen,
  killProcesses,
  listeningPidsForPort,
  navigateBrowserToWorkspace,
  playbackBuildEnv,
  resolveDaemonBinaryPath,
  primeDemoDesktopConnectionToWorkspace,
  primeDemoDesktopConnection,
  resolveBackendPort,
  requireWorkspacePackage,
  runConductor,
  seedProviderRuntimeConfig,
  shouldRecycleSharedCrabNebulaBackend,
  startCrabNebulaStack,
  startSetupProcess,
  openDiffPane,
  openArtifactsPane,
  parseArgs,
  measureTargets,
  captureDiffPaneState,
  captureArtifactsPaneState,
  prepareAutomationAppForLaunch,
  setArtifactsPanePlaybackHidden,
  setDiffPanePlaybackHidden,
  submitComposerPrompt,
  terminateAutomationAppProcesses,
  terminateListeningPort,
  waitForDiffPane,
  waitForDiffPaneReady,
  waitForArtifactsPane,
  waitForProcessReady,
  waitForSessionTurnCompletion,
  waitForTcpPort,
};

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    process.stderr.write(`${String(error?.stack || error)}\n`);
    process.exit(1);
  });
}
