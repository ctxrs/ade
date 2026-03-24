const fs = require("fs");
const path = require("path");

const { waitForTauri } = require("./helpers/tauri.cjs");
const {
  mkTempDir,
  runWizardScenario,
  runCodexComposerSmoke,
  assertConnectedLocalAndListening,
  assertLocalWorkspaceConfig,
  compactEntity,
} = require("./helpers/workspace_wizard_flow.cjs");
const {
  getProviderStatus,
  installProviderAndWait,
  configureOpenRouterEndpoint,
  verifyProviderForWorkspace,
  resolveWorkspaceProviderModelId,
  readOpenRouterEnv,
} = require("./helpers/provider_runtime.cjs");
const { daemonJson } = require("./helpers/daemon.cjs");

const DEFAULT_PROVIDER_ID = "codex";
const reportPath = process.env.CTX_HARNESS_MATRIX_REPORT || path.join("/tmp", "ctx-harness-install-probe-matrix.json");
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
const normalizeErr = (value) => String(value || "").replace(/\s+/g, " ").trim().toLowerCase();

const writeReport = (payload) => {
  try {
    fs.mkdirSync(path.dirname(reportPath), { recursive: true });
    fs.writeFileSync(reportPath, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
  } catch {
    // best effort only
  }
};

describe("harness install/probe matrix (desktop e2e)", () => {
  const runId = `${Date.now()}`;
  const localBase = mkTempDir(`ctx-harness-install-probe-${runId}-`);

  before(async () => {
    await browser.url(`tauri://localhost/workspace-setup?harnessInstallProbeMatrix=${Date.now()}`);
    await waitForTauri();
  });

  after(async () => {
    try {
      fs.rmSync(localBase, { recursive: true, force: true });
    } catch {
      // ignore cleanup failures
    }
  });

  it("covers install failure signal, probe failure signal, and successful recovery", async function () {
    this.timeout(14 * 60_000);
    if (!scenarioEnabled("local-codex-smoke", ["local", "host", "provider"])) this.skip();

    const apiKey = String(process.env.OPENROUTER_API_KEY || "").trim();
    if (!apiKey) {
      console.error("[skip] OPENROUTER_API_KEY is required for harness-install-probe-matrix.spec.cjs");
      this.skip();
    }
    const { baseUrl, modelOverride } = readOpenRouterEnv(DEFAULT_PROVIDER_ID);

    const diagnostics = {
      providerId: DEFAULT_PROVIDER_ID,
      cases: [],
      reportPath,
    };

    const dest = path.join(localBase, "matrix");
    const workspaceId = await runWizardScenario({
      location: "local",
      container: "sandbox",
      network: "providers",
      harnessDownloads: "skip",
      source: { kind: "new", destPath: dest, workspaceName: "harness-install-probe-matrix" },
      setupHook: "",
      mergeQueue: { kind: "skip" },
    });
    diagnostics.workspaceId = workspaceId;

    await assertConnectedLocalAndListening();
    await assertLocalWorkspaceConfig(workspaceId, {
      environment: "sandbox",
      networkMode: "llm_only",
    });

    await installProviderAndWait(DEFAULT_PROVIDER_ID, "container");
    const postInstall = await getProviderStatus(DEFAULT_PROVIDER_ID, "container");
    diagnostics.cases.push({
      id: "fresh-install",
      status: postInstall.health,
      installed: postInstall.installed,
      diagnostics: postInstall.diagnostics,
      details: postInstall.details,
    });
    if (!postInstall.installed) {
      writeReport(diagnostics);
      throw new Error(`provider not installed after install flow: ${JSON.stringify(postInstall)}`);
    }

    let probeFailure = "";
    const invalidEndpointId = await configureOpenRouterEndpoint({
      providerId: DEFAULT_PROVIDER_ID,
      baseUrl: "https://127.0.0.1:9/v1",
      apiKey: "invalid-key",
      modelOverride,
      endpointName: `codex-invalid-${runId}`,
    });
    const invalidRefresh = await daemonJson(
      "POST",
      `/api/providers/${DEFAULT_PROVIDER_ID}/harness_config/endpoints/${encodeURIComponent(invalidEndpointId)}/models/refresh`,
      {},
    );
    const invalidPayloadText = normalizeErr(JSON.stringify(invalidRefresh.payload || {}));
    const invalidLooksFailed = invalidRefresh.status !== 200
      || invalidPayloadText.includes("error")
      || invalidPayloadText.includes("failed")
      || invalidPayloadText.includes("invalid")
      || invalidPayloadText.includes("connect")
      || invalidPayloadText.includes("timeout");
    if (invalidLooksFailed) {
      probeFailure = `status=${invalidRefresh.status} payload=${JSON.stringify(invalidRefresh.payload || null)}`;
    }
    diagnostics.cases.push({
      id: "probe-failure-invalid-endpoint",
      ok: Boolean(probeFailure),
      detail: probeFailure || "unexpected success",
    });
    if (!probeFailure) {
      writeReport(diagnostics);
      throw new Error("expected probe failure for invalid endpoint, but verify succeeded");
    }

    await configureOpenRouterEndpoint({
      providerId: DEFAULT_PROVIDER_ID,
      baseUrl,
      apiKey,
      modelOverride,
      endpointName: `codex-recovery-${runId}`,
    });
    await verifyProviderForWorkspace(workspaceId, DEFAULT_PROVIDER_ID);
    const modelId = await resolveWorkspaceProviderModelId(workspaceId, DEFAULT_PROVIDER_ID, {
      timeoutMs: 90_000,
      pollMs: 3_000,
    });
    diagnostics.cases.push({
      id: "recovery-models-list",
      ok: Boolean(modelId),
      modelId,
    });

    const providersResp = await daemonJson("GET", "/api/providers");
    diagnostics.providers = providersResp.status === 200 && Array.isArray(providersResp.payload)
      ? providersResp.payload
          .filter((entry) => entry && entry.provider_id === DEFAULT_PROVIDER_ID)
          .map((entry) => compactEntity(entry))
      : [{ status: providersResp.status, error: providersResp.error || null }];

    await runCodexComposerSmoke(workspaceId, 240_000);
    diagnostics.cases.push({ id: "first-turn-success", ok: true });

    writeReport(diagnostics);
  });
});
