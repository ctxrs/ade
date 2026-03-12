const assert = require("node:assert/strict");
const fs = require("node:fs");
const http = require("node:http");
const path = require("node:path");

const { waitForTauri } = require("./helpers/tauri.cjs");
const { daemonJson } = require("./helpers/daemon.cjs");
const {
  mkTempDir,
  scenarioEnabled,
  assertConnectedLocalAndListening,
} = require("./helpers/workspace_wizard_flow.cjs");
const { createProviderOAuthHarness } = require("./helpers/provider_oauth_flow.cjs");

const DEFAULT_CASE_TIMEOUT_MS = 5 * 60_000;
const DEFAULT_LOGIN_TIMEOUT_MS = 60_000;

const parsePositiveInt = (raw, fallback) => {
  const parsed = Number.parseInt(String(raw || ""), 10);
  if (!Number.isFinite(parsed) || parsed <= 0) return fallback;
  return parsed;
};

const normalizeText = (value) => String(value || "").trim();

const resolveDesktopDaemonDataDir = () => {
  const raw = normalizeText(process.env.CTX_DESKTOP_DAEMON_DATA_DIR || process.env.CTX_AUTOMATION_INTERNAL_DAEMON_DATA_DIR);
  if (!raw) {
    throw new Error("desktop daemon data dir is not available in automation env");
  }
  if (!path.isAbsolute(raw)) {
    throw new Error(`desktop daemon data dir must be absolute, got '${raw}'`);
  }
  return raw;
};

const agentServerConfigPath = (dataDir) => path.join(
  dataDir,
  "providers",
  "agent-servers",
  "agent_servers.json",
);

const readJsonFile = (filePath) => {
  if (!fs.existsSync(filePath)) return null;
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
};

const writeJsonFile = (filePath, payload) => {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
};

const writeCursorAgentFixture = (baseDir, authUrl) => {
  const scriptPath = path.join(baseDir, "cursor-agent");
  const script = `#!/usr/bin/env node
const fs = require("node:fs");
const capturePath = process.env.CTX_CURSOR_CAPTURE_FILE;
console.log(${JSON.stringify(authUrl)});
console.log("Signed in as cursor-shell-open@example.com");
if (capturePath) {
  fs.appendFileSync(capturePath, JSON.stringify({ event: "captured", service: "cursor-access-token", value: "cursor-shell-open-access-token" }) + "\\n");
  fs.appendFileSync(capturePath, JSON.stringify({ event: "captured", service: "cursor-refresh-token", value: "cursor-shell-open-refresh-token" }) + "\\n");
}
`;
  fs.writeFileSync(scriptPath, script, "utf8");
  fs.chmodSync(scriptPath, 0o755);
  return scriptPath;
};

const startAuthUrlProbeServer = async () => {
  const requests = [];
  let resolveRequest;
  let rejectRequest;
  const requestPromise = new Promise((resolve, reject) => {
    resolveRequest = resolve;
    rejectRequest = reject;
  });
  const server = http.createServer((req, res) => {
    const request = {
      method: normalizeText(req.method) || "GET",
      url: normalizeText(req.url),
    };
    requests.push(request);
    res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
    res.end("<!doctype html><title>ctx cursor oauth probe</title><body>ok</body>");
    if (request.url.startsWith("/cursor/login/device")) {
      resolveRequest?.(request);
      resolveRequest = null;
      rejectRequest = null;
    }
  });

  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  if (!address || typeof address !== "object" || typeof address.port !== "number") {
    throw new Error(`unexpected auth probe server address: ${JSON.stringify(address)}`);
  }

  return {
    authUrl: `http://127.0.0.1:${address.port}/cursor/login/device?code=ctx-shell-open`,
    requests,
    waitForRequest: async (timeoutMs = 15000) => await Promise.race([
      requestPromise,
      new Promise((_, reject) => {
        const timer = setTimeout(() => {
          reject(new Error(`timed out waiting for external browser request; requests=${JSON.stringify(requests)}`));
        }, timeoutMs);
        requestPromise.finally(() => clearTimeout(timer)).catch(() => clearTimeout(timer));
      }),
    ]),
    close: async () => {
      if (!server.listening) return;
      await new Promise((resolve, reject) => {
        server.close((error) => {
          if (error) reject(error);
          else resolve();
        });
      });
      rejectRequest?.(new Error("auth probe server closed before request completed"));
      rejectRequest = null;
      resolveRequest = null;
    },
  };
};

const writeCursorLoginCommandConfig = (dataDir, cursorAgentPath) => {
  const configPath = agentServerConfigPath(dataDir);
  const current = readJsonFile(configPath) || {};
  const next = {
    providers: current.providers && typeof current.providers === "object" ? current.providers : {},
    provider_login_commands:
      current.provider_login_commands && typeof current.provider_login_commands === "object"
        ? current.provider_login_commands
        : {},
    managed_installs:
      current.managed_installs && typeof current.managed_installs === "object" ? current.managed_installs : {},
    managed_provider_targets:
      current.managed_provider_targets && typeof current.managed_provider_targets === "object"
        ? current.managed_provider_targets
        : {},
    managed_install_targets:
      current.managed_install_targets && typeof current.managed_install_targets === "object"
        ? current.managed_install_targets
        : {},
  };
  next.provider_login_commands.cursor = {
    command: cursorAgentPath,
    args: [],
    dependencies: [],
  };
  writeJsonFile(configPath, next);
  return configPath;
};

const clearCursorAccounts = async () => {
  const accountsResp = await daemonJson("GET", "/api/providers/cursor/accounts");
  if (accountsResp.status !== 200) {
    throw new Error(
      `cursor accounts fetch failed (${accountsResp.status}): ${JSON.stringify(accountsResp.payload || null)}`,
    );
  }
  const accountIds = Array.isArray(accountsResp.payload?.accounts)
    ? accountsResp.payload.accounts
      .map((entry) => normalizeText(entry?.id))
      .filter(Boolean)
    : [];
  const clearActiveResp = await daemonJson("PUT", "/api/providers/cursor/active-account", {
    account_id: null,
  });
  if (clearActiveResp.status !== 200) {
    throw new Error(
      `clear cursor active account failed (${clearActiveResp.status}): ${JSON.stringify(clearActiveResp.payload || null)}`,
    );
  }
  for (const accountId of accountIds) {
    const deleteResp = await daemonJson(
      "DELETE",
      `/api/providers/cursor/accounts/${encodeURIComponent(accountId)}`,
    );
    if (deleteResp.status !== 200) {
      throw new Error(
        `delete cursor account ${accountId} failed (${deleteResp.status}): ${JSON.stringify(deleteResp.payload || null)}`,
      );
    }
  }
};

describe("cursor oauth shell open (desktop e2e)", () => {
  const runId = `${Date.now()}`;
  const reportPath = normalizeText(process.env.CTX_AUTOMATION_CURSOR_OAUTH_REPORT)
    || path.join("/tmp", `ctx-cursor-oauth-shell-open-${runId}.json`);
  const localBase = mkTempDir(`ctx-cursor-oauth-shell-open-${runId}-`);

  before(async () => {
    await browser.url(`tauri://localhost/workspace-setup?cursorOauthShellOpen=${runId}`);
    await waitForTauri();
  });

  after(async () => {
    try {
      fs.rmSync(localBase, { recursive: true, force: true });
    } catch {
      // ignore cleanup failures
    }
  });

  it("opens the Cursor auth endpoint through the desktop shell path and completes login", async function () {
    this.timeout(parsePositiveInt(process.env.CTX_AUTOMATION_CASE_TIMEOUT_MS || "", DEFAULT_CASE_TIMEOUT_MS));

    if (!scenarioEnabled("cursor-oauth", ["local", "provider", "oauth", "cursor"])) this.skip();

    const daemonDataDir = resolveDesktopDaemonDataDir();
    const configPath = agentServerConfigPath(daemonDataDir);
    const previousConfig = fs.existsSync(configPath) ? fs.readFileSync(configPath, "utf8") : null;
    const timeoutMs = parsePositiveInt(
      process.env.CTX_AUTOMATION_CURSOR_OAUTH_TIMEOUT_MS || "",
      DEFAULT_LOGIN_TIMEOUT_MS,
    );
    const authProbeServer = await startAuthUrlProbeServer();
    const cursorAgentPath = writeCursorAgentFixture(localBase, authProbeServer.authUrl);

    try {
      await assertConnectedLocalAndListening();
      await clearCursorAccounts();
      writeCursorLoginCommandConfig(daemonDataDir, cursorAgentPath);

      const harness = createProviderOAuthHarness({
        outputPath: reportPath,
        pollMs: 500,
        timeoutMs,
        urlFallbackGraceMs: 0,
      });

      const login = await harness.startProviderLogin("cursor", `cursor-oauth-${runId}`);
      const loginUrl = await harness.awaitLoginUrl(login.loginId, timeoutMs);
      assert.equal(loginUrl.authUrl, authProbeServer.authUrl);
      assert.equal(loginUrl.sanitizedAuthUrl?.path, "/cursor/login/device");

      await harness.openAuthUrl(loginUrl.authUrl);
      const openRequest = await authProbeServer.waitForRequest(timeoutMs);
      assert.equal(openRequest.method, "GET");
      assert.equal(openRequest.url, "/cursor/login/device?code=ctx-shell-open");

      const terminal = await harness.awaitLoginTerminal(login.loginId, timeoutMs);
      assert.equal(terminal.status, "success");
      const activeAccount = await harness.assertAccountActivated("cursor");
      assert.ok(activeAccount.activeAccountId);

      const accountsResp = await daemonJson("GET", "/api/providers/cursor/accounts");
      assert.equal(accountsResp.status, 200);
      assert.equal(accountsResp.payload?.active_account_id, activeAccount.activeAccountId);
    } finally {
      try {
        await clearCursorAccounts();
      } catch {
        // ignore cleanup failures
      }
      try {
        await authProbeServer.close();
      } catch {
        // ignore probe-server cleanup failures
      }
      try {
        if (previousConfig === null) {
          fs.rmSync(configPath, { force: true });
        } else {
          fs.mkdirSync(path.dirname(configPath), { recursive: true });
          fs.writeFileSync(configPath, previousConfig, "utf8");
        }
      } catch {
        // ignore config restore failures
      }
    }
  });
});
