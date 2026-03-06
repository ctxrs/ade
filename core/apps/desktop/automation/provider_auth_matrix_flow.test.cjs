const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const { resolveSubscriptionAuthPlan } = require("./specs/helpers/provider_auth_matrix_flow.cjs");

const mkTempDir = (prefix) => fs.mkdtempSync(path.join(os.tmpdir(), prefix));

test("codex plan skips when OAuth browser credentials are missing", () => {
  const result = resolveSubscriptionAuthPlan("codex", {});
  assert.equal(result.status, "skip");
  assert.match(result.reason, /CTX_E2E_CODEX_OAUTH_EMAIL/i);
});

test("codex plan resolves browser-backed OAuth inputs from env", () => {
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

test("cursor plan resolves managed upsert inputs from env", () => {
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
  const result = resolveSubscriptionAuthPlan("mistral", {});
  assert.equal(result.status, "skip");
  assert.match(result.reason, /CTX_E2E_MISTRAL_HOME_SEED_DIR/);
});
