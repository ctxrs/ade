const fs = require("fs");
const path = require("path");

const { waitForTauri } = require("./helpers/tauri.cjs");
const { daemonJson } = require("./helpers/daemon.cjs");
const { sampleDaemonHealth } = require("./helpers/daemon.cjs");
const {
  mkTempDir,
  initGitRepo,
  assertConnectedLocalAndListening,
  assertLocalWorkspaceConfig,
  runCodexFirstTurnApiSmoke,
} = require("./helpers/workspace_wizard_flow.cjs");
const {
  getProviderStatus,
  installProviderAndWait,
  configureOpenRouterEndpoint,
  verifyProviderForWorkspace,
  resolveWorkspaceProviderModelId,
} = require("./helpers/provider_runtime.cjs");

const DEFAULT_OPENROUTER_BASE_URL = "https://openrouter.ai/api/v1";
const DEFAULT_PROVIDER_ID = "codex";
const DEFAULT_MODEL_OVERRIDE = "openai/gpt-5.2-codex";
const scenarioFilter = new Set(
  String(process.env.CTX_AUTOMATION_SCENARIOS || "")
    .split(",")
    .map((s) => s.trim().toLowerCase())
    .filter(Boolean),
);

const scenarioEnabled = (name, tags = []) => {
  if (scenarioFilter.size === 0) return true;
  return [name, ...tags].some((t) => scenarioFilter.has(String(t).trim().toLowerCase()));
};

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, Math.max(0, Number(ms) || 0)));

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
      if (state && (state !== lastState || pollCount % 10 === 0)) {
        const phaseText = latestPhase?.phase ? ` phase=${latestPhase.phase}` : "";
        const logText = latestLog?.message ? ` log=${String(latestLog.message).slice(0, 220)}` : "";
        log("api.execution.launch.status", `job=${jobId} state=${state}${phaseText} poll=${pollCount}${logText}`);
        lastState = state;
      }
      if (state === "ready") return workspaceId;
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

const requiredOpenRouterEnv = () => {
  const apiKey = String(process.env.OPENROUTER_API_KEY || "").trim();
  const baseUrl = String(process.env.OPENROUTER_BASE_URL || "").trim() || DEFAULT_OPENROUTER_BASE_URL;
  const modelOverride = String(process.env.CTX_E2E_OPENROUTER_MODEL_OVERRIDE || "").trim() || DEFAULT_MODEL_OVERRIDE;
  return { apiKey, baseUrl, modelOverride };
};

describe("container provider OpenRouter (desktop e2e)", () => {
  const runId = `${Date.now()}`;
  const localBase = mkTempDir(`ctx-container-provider-openrouter-${runId}-`);
  const stageLog = (label, details = "") => {
    const suffix = details ? ` ${details}` : "";
    // Keep stage logs concise so failure output points at the exact stuck phase.
    // eslint-disable-next-line no-console
    console.log(`[container-provider-openrouter] ${new Date().toISOString()} ${label}${suffix}`);
  };

  before(async () => {
    await browser.url(`tauri://localhost/workspace-setup?containerProviderOpenrouter=${Date.now()}`);
    await waitForTauri();
  });

  after(async () => {
    try {
      fs.rmSync(localBase, { recursive: true, force: true });
    } catch {
      // ignore cleanup failures
    }
  });

  it("creates container workspace, installs codex, resolves models, and runs first real turn", async function () {
    this.timeout(20 * 60_000);
    if (!scenarioEnabled("local-codex-smoke", ["local", "container", "provider"])) this.skip();

    const { apiKey, baseUrl, modelOverride } = requiredOpenRouterEnv();
    if (!apiKey) {
      console.error("[skip] OPENROUTER_API_KEY is required for container-provider-openrouter.spec.cjs");
      this.skip();
    }

    stageLog("daemon.connect.start");
    await assertConnectedLocalAndListening();
    stageLog("daemon.connect.done");

    const dest = path.join(localBase, "openrouter-codex");
    stageLog("workspace.create.start", `dest=${dest}`);
    const workspaceId = await createAndLaunchContainerWorkspace({
      dest,
      name: "openrouter-codex",
      environment: "container_host_mounted",
      networkMode: "llm_only",
      log: stageLog,
    });
    stageLog("workspace.create.done", `workspaceId=${workspaceId}`);

    await assertLocalWorkspaceConfig(workspaceId, {
      environment: "container_host_mounted",
      networkMode: "llm_only",
    });

    stageLog("install.start", DEFAULT_PROVIDER_ID);
    await installProviderAndWait(DEFAULT_PROVIDER_ID, "container");
    const preStatus = await getProviderStatus(DEFAULT_PROVIDER_ID);
    stageLog("install.done", `installed=${preStatus.installed} health=${preStatus.health}`);
    if (!preStatus.installed) {
      throw new Error(`provider did not report installed after install: ${JSON.stringify(preStatus)}`);
    }

    stageLog("endpoint.upsert.start");
    await configureOpenRouterEndpoint({
      providerId: DEFAULT_PROVIDER_ID,
      baseUrl,
      apiKey,
      modelOverride,
      endpointName: `codex-openrouter-${runId}`,
    });
    stageLog("endpoint.upsert.done");
    stageLog("provider.verify.start");
    await verifyProviderForWorkspace(workspaceId, DEFAULT_PROVIDER_ID);
    stageLog("provider.verify.done");
    stageLog("model.resolve.start");
    const modelId = await resolveWorkspaceProviderModelId(workspaceId, DEFAULT_PROVIDER_ID, {
      timeoutMs: 90_000,
      pollMs: 3_000,
    });
    stageLog("model.resolve.done", `model=${modelId}`);
    if (!modelId || !String(modelId).trim()) {
      throw new Error("provider options returned empty model id after OpenRouter config");
    }

    stageLog("composer.smoke.start");
    await runCodexFirstTurnApiSmoke(workspaceId, {
      providerId: DEFAULT_PROVIDER_ID,
      modelId,
      prompt: "hello",
    }, 240_000);
    stageLog("composer.smoke.done");
    stageLog("health.sample.start");
    const health = await sampleDaemonHealth({ durationMs: 15_000, intervalMs: 2_000 });
    stageLog("health.sample.done", `ok=${health.ok}`);
    if (!health.ok) {
      throw new Error(`daemon became unhealthy after first-turn flow: ${JSON.stringify(health.failures)}`);
    }
  });
});
