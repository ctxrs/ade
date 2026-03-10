const fs = require("fs");
const os = require("os");
const path = require("path");
const crypto = require("crypto");
const { spawnSync } = require("child_process");

const { getConnectionInfo } = require("./tauri.cjs");
const { daemonJson, safeDaemonJson } = require("./daemon.cjs");
const {
  initGitRepo,
  collectWorkspaceRouteDiagnostics,
  collectCodexSmokeDiagnostics,
} = require("./workspace_wizard_flow.cjs");

const PODMAN_MACHINE_PREFIX = "ctx";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, Math.max(0, Number(ms) || 0)));

const parseJsonFile = (filePath) => {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch {
    return null;
  }
};

const desktopOsToken = () => {
  if (process.platform === "darwin") return "macos";
  if (process.platform === "win32") return "windows";
  return "linux";
};

const desktopArchToken = () => {
  if (process.arch === "arm64") return "aarch64";
  if (process.arch === "x64") return "x86_64";
  return process.arch;
};

const podmanDataRootHash = (dataRoot) => {
  // Match ctx-http harness runtime: first 6 bytes of SHA-256 rendered as 12 hex chars.
  return crypto.createHash("sha256").update(String(dataRoot || "")).digest("hex").slice(0, 12);
};

const ctxPodmanMachineName = (dataRoot) => `${PODMAN_MACHINE_PREFIX}-${podmanDataRootHash(dataRoot)}`;

const podmanRuntimeRoot = (dataRoot) => {
  const hash = podmanDataRootHash(dataRoot);
  if (process.platform === "win32") {
    return path.join(os.tmpdir(), "ctxp", hash);
  }
  return path.join("/tmp", "ctxp", hash);
};

const podmanHomeRoot = (dataRoot) => path.join(podmanRuntimeRoot(dataRoot), "home");
const podmanTempRoot = (dataRoot) => path.join(podmanRuntimeRoot(dataRoot), "tmp");

const ensureDir = (dirPath) => {
  fs.mkdirSync(dirPath, { recursive: true });
};

const resolveBundledPodmanPath = () => {
  const bundleDir = String(process.env.CTX_BUNDLE_DIR || "").trim();
  if (!bundleDir) return "";
  const manifestPath = path.join(bundleDir, "manifest.json");
  if (!fs.existsSync(manifestPath)) return "";
  const manifest = parseJsonFile(manifestPath);
  const runtimes = Array.isArray(manifest?.runtimes) ? manifest.runtimes : [];
  const runtime = runtimes.find((entry) =>
    entry
    && entry.id === "podman"
    && entry.os === desktopOsToken()
    && entry.arch === desktopArchToken()
    && typeof entry.root === "string"
    && entry.root.trim()
    && typeof entry.bin === "string"
    && entry.bin.trim(),
  );
  if (!runtime) return "";
  const podmanBin = path.join(bundleDir, runtime.root, runtime.bin);
  return fs.existsSync(podmanBin) ? podmanBin : "";
};

const resolveManagedPodmanPath = (dataRoot) => {
  const runtimeBase = path.join(
    dataRoot,
    "managed",
    "runtimes",
    "podman",
    desktopOsToken(),
    desktopArchToken(),
  );
  if (!fs.existsSync(runtimeBase)) return "";
  const versionDirs = fs.readdirSync(runtimeBase, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort()
    .reverse();
  const binName = process.platform === "win32" ? "podman.exe" : "podman";
  for (const versionDir of versionDirs) {
    const candidate = path.join(runtimeBase, versionDir, "usr", "bin", binName);
    if (fs.existsSync(candidate)) return candidate;
  }
  return "";
};

const resolveDesktopDaemonDataDir = () => {
  const raw = String(process.env.CTX_DESKTOP_DAEMON_DATA_DIR || "").trim();
  if (!raw) {
    throw new Error("CTX_DESKTOP_DAEMON_DATA_DIR is not set; desktop automation did not expose the local daemon data dir");
  }
  if (!path.isAbsolute(raw)) {
    throw new Error(`CTX_DESKTOP_DAEMON_DATA_DIR must be absolute, got '${raw}'`);
  }
  return raw;
};

const resolveDesktopPodmanControl = () => {
  const dataRoot = resolveDesktopDaemonDataDir();
  const explicit = String(process.env.CTX_PODMAN_PATH || "").trim();
  const bundled = resolveBundledPodmanPath();
  const managed = resolveManagedPodmanPath(dataRoot);
  const bin = [explicit, bundled, managed].find((candidate) => candidate && fs.existsSync(candidate));
  if (!bin) {
    throw new Error(
      `podman binary unavailable for desktop lifecycle automation; checked CTX_PODMAN_PATH, bundle manifest under ${String(process.env.CTX_BUNDLE_DIR || "").trim() || "<unset>"}, and managed runtime under ${dataRoot}`,
    );
  }

  const xdgRoot = path.join(dataRoot, "podman", "xdg");
  const xdgConfig = path.join(xdgRoot, "config");
  const xdgData = path.join(xdgRoot, "data");
  const xdgRun = path.join(xdgRoot, "run");
  const podmanHome = podmanHomeRoot(dataRoot);
  const podmanTmp = podmanTempRoot(dataRoot);
  for (const dirPath of [xdgConfig, xdgData, xdgRun, podmanHome, podmanTmp]) {
    ensureDir(dirPath);
  }

  return {
    bin,
    dataRoot,
    machineName: ctxPodmanMachineName(dataRoot),
    env: {
      XDG_CONFIG_HOME: xdgConfig,
      XDG_DATA_HOME: xdgData,
      XDG_RUNTIME_DIR: xdgRun,
      HOME: podmanHome,
      TMPDIR: podmanTmp,
      TMP: podmanTmp,
      TEMP: podmanTmp,
    },
  };
};

const runPodman = (
  control,
  args,
  { timeoutMs = 120000, allowFailure = false } = {},
) => {
  const result = spawnSync(control.bin, args, {
    encoding: "utf8",
    timeout: timeoutMs,
    env: {
      ...process.env,
      ...control.env,
    },
  });
  const stdout = String(result.stdout || "").trim();
  const stderr = String(result.stderr || "").trim();
  const combined = [stderr, stdout].filter(Boolean).join("\n").trim();
  if (result.error) {
    throw new Error(`podman ${args.join(" ")} failed: ${result.error.message}`);
  }
  if (!allowFailure && result.status !== 0) {
    throw new Error(
      `podman ${args.join(" ")} failed (${result.status ?? "null"}): ${combined || "<no output>"}`,
    );
  }
  return {
    args: [...args],
    status: result.status ?? null,
    signal: result.signal ?? null,
    stdout,
    stderr,
    combined,
  };
};

const waitForPodmanReachability = async (
  control,
  expectedReachable,
  { timeoutMs = 180000, pollMs = 1000, infoTimeoutMs = 15000 } = {},
) => {
  const startedAt = Date.now();
  let lastResult = null;
  while (Date.now() - startedAt < timeoutMs) {
    lastResult = runPodman(control, ["info"], {
      allowFailure: true,
      timeoutMs: infoTimeoutMs,
    });
    const reachable = lastResult.status === 0;
    if (reachable === Boolean(expectedReachable)) {
      return {
        reachable,
        elapsedMs: Date.now() - startedAt,
        lastResult,
      };
    }
    await sleep(pollMs);
  }
  throw new Error(
    `podman reachability did not become ${expectedReachable ? "ready" : "stopped"} in time; last=${JSON.stringify(lastResult)}`,
  );
};

const stopPodmanMachineAndWait = async (
  control,
  { timeoutMs = 180000 } = {},
) => {
  const stop = runPodman(control, ["machine", "stop", control.machineName], {
    allowFailure: true,
    timeoutMs,
  });
  const lower = String(stop.combined || "").toLowerCase();
  const acceptableFailure = stop.status === 0
    || lower.includes("already stopped")
    || lower.includes("vm is already stopped");
  if (!acceptableFailure) {
    throw new Error(`podman machine stop failed: ${stop.combined || "<no output>"}`);
  }
  const readiness = await waitForPodmanReachability(control, false, { timeoutMs });
  return { stop, readiness };
};

const startPodmanMachineAndWait = async (
  control,
  { timeoutMs = 240000 } = {},
) => {
  const start = runPodman(control, ["machine", "start", control.machineName], {
    allowFailure: true,
    timeoutMs,
  });
  const lower = String(start.combined || "").toLowerCase();
  const acceptableFailure = start.status === 0
    || lower.includes("already running")
    || lower.includes("running but not yet ready")
    || lower.includes("waiting for readiness")
    || lower.includes("connection refused");
  if (!(start.status === 0 || acceptableFailure)) {
    throw new Error(`podman machine start failed: ${start.combined || "<no output>"}`);
  }
  const readiness = await waitForPodmanReachability(control, true, { timeoutMs });
  return { start, readiness };
};

const tauriInvoke = async (command, args) => {
  let result;
  try {
    result = await browser.executeAsync(({ cmd, payload }, done) => {
      const tauriInvoke = window.__TAURI__?.core?.invoke;
      const internalsInvoke = window.__TAURI_INTERNALS__?.invoke;
      const invoke = internalsInvoke || tauriInvoke;
      if (!invoke) {
        done({ error: "Tauri invoke API not available" });
        return;
      }
      Promise.resolve()
        .then(() => invoke(cmd, payload))
        .then((value) => done({ value }))
        .catch((error) => done({ error: String(error) }));
    }, { cmd: command, payload: args });
  } catch (error) {
    return { error: String(error) };
  }

  if (result && typeof result === "object" && Object.prototype.hasOwnProperty.call(result, "error")) {
    return { error: String(result.error || "") };
  }
  return { value: result ? result.value : undefined };
};

const waitForDesktopAppReady = async (timeoutMs = 120000) => {
  let lastError = "";
  await browser.waitUntil(async () => {
    try {
      const ready = await browser.execute(() => {
        const invoke = window.__TAURI__?.core?.invoke || window.__TAURI_INTERNALS__?.invoke;
        return Boolean(invoke) && document.readyState !== "loading";
      });
      return Boolean(ready);
    } catch (error) {
      lastError = String(error);
      return false;
    }
  }, {
    timeout: timeoutMs,
    interval: 500,
    timeoutMsg: `desktop app did not become ready after restart: ${lastError || "unknown error"}`,
  });
};

const restartLocalDaemonAndWait = async (
  { timeoutMs = 90000 } = {},
) => {
  const response = await tauriInvoke("desktop_restart_local_daemon", { req: { confirm: true } });
  if (response.error) {
    throw new Error(`desktop_restart_local_daemon failed: ${response.error}`);
  }
  let lastHealth = null;
  await browser.waitUntil(async () => {
    lastHealth = await safeDaemonJson("GET", "/api/health");
    return Number(lastHealth.status || 0) === 200 && !lastHealth.error;
  }, {
    timeout: timeoutMs,
    interval: 500,
    timeoutMsg: `local daemon did not become healthy after restart: ${JSON.stringify(lastHealth)}`,
  });
  return response.value || null;
};

const restartDesktopAppAndWait = async (
  { timeoutMs = 120000 } = {},
) => {
  const response = await tauriInvoke("desktop_restart_app", {});
  if (response.error) {
    throw new Error(`desktop_restart_app failed: ${response.error}`);
  }
  await sleep(500);
  await waitForDesktopAppReady(timeoutMs);
  return response.value || null;
};

const openWorkspaceRouteAndWait = async (
  workspaceId,
  { timeoutMs = 60000 } = {},
) => {
  const id = String(workspaceId || "").trim();
  if (!id) {
    throw new Error("workspaceId is required");
  }
  await waitForDesktopAppReady();
  await browser.execute((targetId, nonce) => {
    window.location.href = `/workspaces/${targetId}?ctxLifecycleResume=${nonce}`;
  }, id, Date.now());

  let lastPath = "";
  await browser.waitUntil(async () => {
    try {
      lastPath = await browser.execute(() => window.location.pathname);
    } catch (error) {
      lastPath = `<error: ${String(error)}>`;
      return false;
    }
    return lastPath === `/workspaces/${id}`;
  }, {
    timeout: timeoutMs,
    interval: 250,
    timeoutMsg: `workspace route did not load for ${id}; last_path=${lastPath}`,
  });
  return lastPath;
};

const createAndLaunchContainerWorkspace = async ({
  dest,
  name,
  environment,
  networkMode,
  timeoutMs = 15 * 60_000,
  log = () => {},
}) => {
  initGitRepo(dest, name);
  log("api.workspaces.create.request", `name=${name}`);
  const create = await daemonJson("POST", "/api/workspaces", {
    root_path: dest,
    name,
  });
  log("api.workspaces.create.response", `status=${create.status}`);
  if (create.status !== 200) {
    throw new Error(`workspace create failed (${create.status}): ${JSON.stringify(create.payload || null)}`);
  }
  const workspaceId = String(create.payload?.id || "").trim();
  if (!workspaceId) {
    throw new Error(`workspace create response missing id: ${JSON.stringify(create.payload || null)}`);
  }

  log("api.workspaces.execution_config.request", `workspace=${workspaceId}`);
  const setExec = await daemonJson("POST", `/api/workspaces/${workspaceId}/execution_config`, {
    environment,
    network_mode: networkMode,
  });
  log("api.workspaces.execution_config.response", `status=${setExec.status}`);
  if (setExec.status !== 200) {
    throw new Error(`execution config update failed (${setExec.status}): ${JSON.stringify(setExec.payload || null)}`);
  }

  log("api.execution.launch.start.request", `workspace=${workspaceId}`);
  const launch = await daemonJson("POST", "/api/execution/launch/start", {
    workspace_id: workspaceId,
  });
  log("api.execution.launch.start.response", `status=${launch.status}`);
  if (launch.status !== 200) {
    throw new Error(`execution launch start failed (${launch.status}): ${JSON.stringify(launch.payload || null)}`);
  }
  const jobId = String(launch.payload?.job_id || "").trim();
  if (!jobId) {
    throw new Error(`execution launch response missing job_id: ${JSON.stringify(launch.payload || null)}`);
  }
  log("api.execution.launch.job", `job=${jobId}`);

  const startedAt = Date.now();
  let lastState = "";
  let lastPayload = null;
  let pollCount = 0;
  const transitions = [];
  while (Date.now() - startedAt < timeoutMs) {
    const status = await daemonJson("GET", `/api/execution/launch/status?job_id=${encodeURIComponent(jobId)}`);
    if (status.status === 200) {
      lastPayload = status.payload || null;
      const state = String(status.payload?.state || "").trim().toLowerCase();
      const phases = Array.isArray(status.payload?.phases) ? status.payload.phases : [];
      const latestPhase = phases.length > 0 ? phases[phases.length - 1] : null;
      const logs = Array.isArray(status.payload?.logs) ? status.payload.logs : [];
      const latestLog = logs.length > 0 ? logs[logs.length - 1] : null;
      pollCount += 1;
      if (state && state !== lastState) {
        transitions.push({
          ts: new Date().toISOString(),
          state,
          latest_phase: latestPhase || null,
          latest_log: latestLog || null,
        });
      }
      if (state && (state !== lastState || pollCount % 10 === 0)) {
        const phaseText = latestPhase?.phase ? ` phase=${latestPhase.phase}` : "";
        const logText = latestLog?.message ? ` log=${String(latestLog.message).slice(0, 220)}` : "";
        log("api.execution.launch.status", `job=${jobId} state=${state}${phaseText} poll=${pollCount}${logText}`);
        lastState = state;
      }
      if (state === "ready") {
        return {
          workspaceId,
          jobId,
          finalStatus: lastPayload,
          transitions,
        };
      }
      if (state === "error") {
        throw new Error(`execution launch failed: ${JSON.stringify(status.payload || null)}`);
      }
    } else {
      log("api.execution.launch.status.error", `job=${jobId} status=${status.status}`);
    }
    await sleep(1000);
  }
  throw new Error(
    `execution launch timed out for workspace=${workspaceId}, job=${jobId}, last=${JSON.stringify(lastPayload)}`,
  );
};

const safeGetConnectionInfo = async () => {
  try {
    return await getConnectionInfo();
  } catch (error) {
    return { error: String(error) };
  }
};

const collectPodmanDiagnostics = (control) => {
  if (!control) return null;
  const machineList = runPodman(control, ["machine", "list"], { allowFailure: true, timeoutMs: 15000 });
  const info = runPodman(control, ["info"], { allowFailure: true, timeoutMs: 15000 });
  const ps = runPodman(control, ["ps", "-a", "--format", "json"], { allowFailure: true, timeoutMs: 15000 });
  return {
    bin: control.bin,
    machine_name: control.machineName,
    machine_list: machineList,
    info,
    containers: ps,
  };
};

const collectLifecycleDiagnostics = async (workspaceId, control = null) => {
  const id = String(workspaceId || "").trim();
  const health = await safeDaemonJson("GET", "/api/health");
  const connection = await safeGetConnectionInfo();
  const route = await collectWorkspaceRouteDiagnostics().catch((error) => ({ error: String(error) }));
  const workspace = id
    ? await safeDaemonJson("GET", `/api/workspaces/${id}`)
    : null;
  const executionConfig = id
    ? await safeDaemonJson("GET", `/api/workspaces/${id}/execution_config`)
    : null;
  const harnessContainer = id
    ? await safeDaemonJson("GET", `/api/workspaces/${id}/harness_container`)
    : null;
  const smoke = id
    ? await collectCodexSmokeDiagnostics(id).catch((error) => ({ error: String(error) }))
    : null;
  return {
    captured_at: new Date().toISOString(),
    connection,
    health,
    route,
    workspace,
    executionConfig,
    harnessContainer,
    smoke,
    podman: collectPodmanDiagnostics(control),
  };
};

const writeLifecycleReport = (reportPath, payload) => {
  fs.mkdirSync(path.dirname(reportPath), { recursive: true });
  fs.writeFileSync(reportPath, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
};

module.exports = {
  ctxPodmanMachineName,
  resolveDesktopPodmanControl,
  runPodman,
  stopPodmanMachineAndWait,
  startPodmanMachineAndWait,
  tauriInvoke,
  waitForDesktopAppReady,
  restartLocalDaemonAndWait,
  restartDesktopAppAndWait,
  openWorkspaceRouteAndWait,
  createAndLaunchContainerWorkspace,
  collectPodmanDiagnostics,
  collectLifecycleDiagnostics,
  writeLifecycleReport,
};
