const fs = require("node:fs");
const path = require("node:path");

const { waitForTauri } = require("./helpers/tauri.cjs");
const { daemonJson } = require("./helpers/daemon.cjs");
const {
  mkTempDir,
  initGitRepo,
  scenarioEnabled,
  assertConnectedLocalAndListening,
} = require("./helpers/workspace_wizard_flow.cjs");
const {
  verifyProviderForWorkspace,
  resolveWorkspaceProviderModelId,
} = require("./helpers/provider_runtime.cjs");
const {
  createProviderOAuthHarness,
} = require("./helpers/provider_oauth_flow.cjs");

const DEFAULT_CASE_TIMEOUT_MS = 20 * 60_000;
const DEFAULT_LOGIN_TIMEOUT_MS = 15 * 60_000;

const parsePositiveInt = (raw, fallback) => {
  const parsed = Number.parseInt(String(raw || ""), 10);
  if (!Number.isFinite(parsed) || parsed <= 0) return fallback;
  return parsed;
};

const waitMs = (ms) => new Promise((resolve) => setTimeout(resolve, Math.max(0, Number(ms) || 0)));

const normalizeText = (value) => String(value || "").trim();

const writeSkipReport = (reportPath, reason) => {
  if (!reportPath) return;
  fs.mkdirSync(path.dirname(reportPath), { recursive: true });
  fs.writeFileSync(reportPath, `${JSON.stringify({
    schema_version: 1,
    result: "skipped",
    reason: normalizeText(reason),
    completed_at: new Date().toISOString(),
  }, null, 2)}\n`, "utf8");
};

const startCodexDesktopRelay = async ({ loginId, callbackUrl, completionToken }) => {
  const result = await browser.execute(async (req) => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return { ok: false, error: "Tauri invoke not available" };
    try {
      const accepted = await invoke("desktop_start_codex_login_relay", { req });
      return { ok: Boolean(accepted) };
    } catch (error) {
      return { ok: false, error: String(error) };
    }
  }, {
    login_id: loginId,
    callback_url: callbackUrl,
    completion_token: completionToken,
  });
  if (!result || result.ok !== true) {
    throw new Error(normalizeText(result?.error) || "failed to start codex desktop relay");
  }
};

const createWorkspaceAndLaunchExecution = async ({ baseDir, name }) => {
  const dest = path.join(baseDir, name.replace(/[^a-zA-Z0-9._-]+/g, "-"));
  initGitRepo(dest, name);

  const create = await daemonJson("POST", "/api/workspaces", {
    root_path: dest,
    name,
  });
  if (create.status !== 200) {
    throw new Error(`workspace create failed (${create.status}): ${JSON.stringify(create.payload || null)}`);
  }
  const workspaceId = normalizeText(create.payload?.id);
  if (!workspaceId) {
    throw new Error(`workspace create response missing id: ${JSON.stringify(create.payload || null)}`);
  }

  const setExec = await daemonJson("POST", `/api/workspaces/${workspaceId}/execution_config`, {
    environment: "host",
    network_mode: "all",
  });
  if (setExec.status !== 200) {
    throw new Error(`execution config update failed (${setExec.status}): ${JSON.stringify(setExec.payload || null)}`);
  }

  const launch = await daemonJson("POST", "/api/execution/launch/start", {
    workspace_id: workspaceId,
  });
  if (launch.status !== 200) {
    throw new Error(`execution launch start failed (${launch.status}): ${JSON.stringify(launch.payload || null)}`);
  }
  const jobId = normalizeText(launch.payload?.job_id);
  if (!jobId) {
    throw new Error(`execution launch response missing job_id: ${JSON.stringify(launch.payload || null)}`);
  }

  const deadline = Date.now() + 5 * 60_000;
  let lastPayload = null;
  while (Date.now() <= deadline) {
    const status = await daemonJson("GET", `/api/execution/launch/status?job_id=${encodeURIComponent(jobId)}`);
    if (status.status === 200) {
      lastPayload = status.payload || null;
      const state = normalizeText(status.payload?.state).toLowerCase();
      if (state === "ready") {
        return { workspaceId, dest };
      }
      if (state === "error") {
        throw new Error(`execution launch failed: ${JSON.stringify(status.payload || null)}`);
      }
    }
    await waitMs(1000);
  }

  throw new Error(`execution launch timed out for workspace=${workspaceId}; last=${JSON.stringify(lastPayload)}`);
};

describe("codex oauth harness framework (desktop e2e)", () => {
  const runId = `${Date.now()}`;
  const reportPath = normalizeText(process.env.CTX_AUTOMATION_CODEX_OAUTH_REPORT)
    || path.join("/tmp", `ctx-codex-oauth-framework-${runId}.json`);
  const localBase = mkTempDir(`ctx-codex-oauth-framework-${runId}-`);

  before(async () => {
    await browser.url(`tauri://localhost/workspace-setup?codexOauthFramework=${runId}`);
    await waitForTauri();
  });

  after(async () => {
    try {
      fs.rmSync(localBase, { recursive: true, force: true });
    } catch {
      // ignore cleanup failures
    }
  });

  it("drives the Codex subscription login lifecycle through the shared oauth helper", async function () {
    this.timeout(parsePositiveInt(process.env.CTX_AUTOMATION_CASE_TIMEOUT_MS || "", DEFAULT_CASE_TIMEOUT_MS));

    if (!scenarioEnabled("local-codex-smoke", ["local", "provider", "oauth"])) this.skip();

    const enabled = ["1", "true", "yes"].includes(
      normalizeText(process.env.CTX_AUTOMATION_CODEX_OAUTH_ENABLE).toLowerCase(),
    );
    if (!enabled) {
      const reason =
        "CTX_AUTOMATION_CODEX_OAUTH_ENABLE=1 is required for codex-oauth-framework.spec.cjs; this flow needs real external Codex/ChatGPT browser credentials and is intentionally gated when they are absent.";
      console.error(`[skip] ${reason}`);
      writeSkipReport(reportPath, reason);
      this.skip();
    }

    const oauthHarness = createProviderOAuthHarness({
      outputPath: reportPath,
      pollMs: 1000,
      timeoutMs: parsePositiveInt(process.env.CTX_AUTOMATION_CODEX_OAUTH_TIMEOUT_MS || "", DEFAULT_LOGIN_TIMEOUT_MS),
      urlFallbackGraceMs: 2000,
    });

    let workspaceId = "";
    try {
      await assertConnectedLocalAndListening();

      const login = await oauthHarness.startProviderLogin(
        "codex",
        normalizeText(process.env.CTX_AUTOMATION_CODEX_OAUTH_LABEL) || `codex-oauth-${runId}`,
      );

      if (login.expectedCallbackUrl && login.completionToken) {
        await startCodexDesktopRelay({
          loginId: login.loginId,
          callbackUrl: login.expectedCallbackUrl,
          completionToken: login.completionToken,
        });
      }

      const authUrl = await oauthHarness.awaitLoginUrl(login.loginId, 20_000);
      await oauthHarness.openAuthUrl(authUrl.authUrl);

      const terminal = await oauthHarness.awaitLoginTerminal(
        login.loginId,
        parsePositiveInt(process.env.CTX_AUTOMATION_CODEX_OAUTH_TIMEOUT_MS || "", DEFAULT_LOGIN_TIMEOUT_MS),
      );
      if (terminal.status !== "success") {
        throw new Error(`codex oauth login did not succeed: ${JSON.stringify(terminal.redactedPayload || terminal)}`);
      }

      await oauthHarness.assertAccountActivated("codex");

      const workspace = await createWorkspaceAndLaunchExecution({
        baseDir: localBase,
        name: `codex-oauth-framework-${runId}`,
      });
      workspaceId = workspace.workspaceId;

      await verifyProviderForWorkspace(workspace.workspaceId, "codex");
      await resolveWorkspaceProviderModelId(workspace.workspaceId, "codex", {
        timeoutMs: 90_000,
        pollMs: 3000,
      });
    } finally {
      if (workspaceId) {
        await daemonJson("DELETE", `/api/workspaces/${workspaceId}`);
      }
    }
  });
});
