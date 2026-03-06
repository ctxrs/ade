const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const HELPER_PATH = require.resolve("./specs/helpers/provider_auth_matrix_flow.cjs");
const DAEMON_HELPER_PATH = require.resolve("./specs/helpers/daemon.cjs");
const PROVIDER_RUNTIME_HELPER_PATH = require.resolve("./specs/helpers/provider_runtime.cjs");
const OAUTH_HELPER_PATH = require.resolve("./specs/helpers/provider_oauth_flow.cjs");

const mkTempDir = (prefix) => fs.mkdtempSync(path.join(os.tmpdir(), prefix));

const clearHelperCaches = () => {
  delete require.cache[HELPER_PATH];
  delete require.cache[DAEMON_HELPER_PATH];
  delete require.cache[PROVIDER_RUNTIME_HELPER_PATH];
  delete require.cache[OAUTH_HELPER_PATH];
};

const loadHelper = ({
  daemonJson = async () => {
    throw new Error("unexpected daemonJson call");
  },
  selectSubscriptionSource = async () => {
    throw new Error("unexpected selectSubscriptionSource call");
  },
  completeCodexOauthWithBrowserCredentials = async () => {
    throw new Error("unexpected completeCodexOauthWithBrowserCredentials call");
  },
} = {}) => {
  clearHelperCaches();
  require.cache[DAEMON_HELPER_PATH] = {
    id: DAEMON_HELPER_PATH,
    filename: DAEMON_HELPER_PATH,
    loaded: true,
    exports: { daemonJson },
  };
  require.cache[PROVIDER_RUNTIME_HELPER_PATH] = {
    id: PROVIDER_RUNTIME_HELPER_PATH,
    filename: PROVIDER_RUNTIME_HELPER_PATH,
    loaded: true,
    exports: { selectSubscriptionSource },
  };
  require.cache[OAUTH_HELPER_PATH] = {
    id: OAUTH_HELPER_PATH,
    filename: OAUTH_HELPER_PATH,
    loaded: true,
    exports: { completeCodexOauthWithBrowserCredentials },
  };
  return require(HELPER_PATH);
};

test.afterEach(() => {
  clearHelperCaches();
  delete process.env.CTX_E2E_CODEX_OAUTH_EMAIL;
  delete process.env.CTX_E2E_CODEX_OAUTH_PASSWORD;
  delete process.env.CTX_E2E_CODEX_OAUTH_TOTP_SECRET;
});

test("codex plan skips when OAuth browser credentials are missing", () => {
  const { resolveSubscriptionAuthPlan } = loadHelper();
  const result = resolveSubscriptionAuthPlan("codex", {});
  assert.equal(result.status, "skip");
  assert.match(result.reason, /CTX_E2E_CODEX_OAUTH_EMAIL/i);
});

test("codex plan resolves browser-backed OAuth inputs from env", () => {
  const { resolveSubscriptionAuthPlan } = loadHelper();
  const result = resolveSubscriptionAuthPlan(
    "codex",
    {
      CTX_E2E_CODEX_OAUTH_EMAIL: "user@example.com",
      CTX_E2E_CODEX_OAUTH_PASSWORD: "super-secret-password",
      CTX_E2E_CODEX_OAUTH_TOTP_SECRET: "JBSWY3DPEHPK3PXP",
    },
  );
  assert.equal(result.status, "ready");
  assert.equal(result.plan.strategy, "codex_oauth_browser");
  assert.equal(result.plan.email, "user@example.com");
  assert.equal(result.plan.password, "super-secret-password");
  assert.equal(result.plan.totpSecret, "JBSWY3DPEHPK3PXP");
});

test("prepareSubscriptionAuth returns skip with blocker artifacts when codex oauth is blocked", async () => {
  process.env.CTX_E2E_CODEX_OAUTH_EMAIL = "user@example.com";
  process.env.CTX_E2E_CODEX_OAUTH_PASSWORD = "super-secret-password";
  process.env.CTX_E2E_CODEX_OAUTH_TOTP_SECRET = "JBSWY3DPEHPK3PXP";

  let oauthCalls = 0;
  let subscriptionSourceCalled = false;
  const { prepareSubscriptionAuth } = loadHelper({
    completeCodexOauthWithBrowserCredentials: async () => {
      oauthCalls += 1;
      return {
        status: "blocked",
        providerId: "codex",
        loginId: "acct-codex",
        authUrl: {
          scheme: "https",
          host: "chat.openai.com",
          path: "/challenge",
          redacted: "https://chat.openai.com/challenge",
        },
        browserFlow: {
          status: "blocked",
          blocked_reason: "email_challenge_required",
          message: "OpenAI auth requires an email verification code challenge that automation cannot satisfy deterministically",
          challenge: {
            kind: "email_challenge_required",
            state: {
              title: "Check your inbox",
            },
          },
        },
        loginStatus: {
          status: "pending",
        },
      };
    },
    selectSubscriptionSource: async () => {
      subscriptionSourceCalled = true;
      return { source: "subscription" };
    },
  });

  const result = await prepareSubscriptionAuth({
    providerId: "codex",
    envTarget: "local_host",
  });

  assert.equal(oauthCalls, 1);
  assert.equal(result.status, "skip");
  assert.equal(result.reason, "codex oauth blocked: email_challenge_required");
  assert.equal(result.artifacts.oauth_login.status, "blocked");
  assert.equal(result.artifacts.oauth_login.browserFlow.blocked_reason, "email_challenge_required");
  assert.equal(subscriptionSourceCalled, false);
});

test("cursor plan resolves managed upsert inputs from env", () => {
  const { resolveSubscriptionAuthPlan } = loadHelper();
  const result = resolveSubscriptionAuthPlan("cursor", {
    CTX_E2E_CURSOR_API_KEY: "cursor-key",
    CTX_E2E_CURSOR_EMAIL: "dev@example.com",
  });
  assert.equal(result.status, "ready");
  assert.equal(result.plan.strategy, "managed_upsert");
  assert.equal(result.plan.endpoint, "/api/providers/cursor/accounts");
  assert.equal(result.plan.body.token, "cursor-key");
  assert.equal(result.plan.body.email, "dev@example.com");
});

test("amp plan accepts secrets JSON file input", () => {
  const { resolveSubscriptionAuthPlan } = loadHelper();
  const dir = mkTempDir("ctx-provider-auth-amp-");
  const secretsPath = path.join(dir, "secrets.json");
  fs.writeFileSync(secretsPath, '{"apiKey@test":"secret"}', "utf8");
  const result = resolveSubscriptionAuthPlan("amp", {
    CTX_E2E_AMP_SECRETS_PATH: secretsPath,
  });
  assert.equal(result.status, "ready");
  assert.equal(result.plan.strategy, "stage_amp_secrets");
  assert.equal(result.plan.sources.secrets_json, secretsPath);
});

test("mistral plan requires a seeded runtime home directory", () => {
  const { resolveSubscriptionAuthPlan } = loadHelper();
  const result = resolveSubscriptionAuthPlan("mistral", {});
  assert.equal(result.status, "skip");
  assert.match(result.reason, /CTX_E2E_MISTRAL_HOME_SEED_DIR/);
});
