const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const childProcess = require("node:child_process");

const scriptPath = path.join(__dirname, "desktop_e2e_preflight.cjs");

const run = (args, env = {}) =>
  childProcess.spawnSync("node", [scriptPath, ...args], {
    cwd: path.join(__dirname, ".."),
    encoding: "utf8",
    env: {
      ...process.env,
      ...env,
    },
  });

const writeUnreadableFile = (filePath, contents) => {
  fs.writeFileSync(filePath, contents, "utf8");
  fs.chmodSync(filePath, 0o000);
};

test("strict preflight fails when a required provider-auth secret is missing", () => {
  const result = run([
    "--suite",
    "provider-auth-matrix-required",
    "--platform",
    "darwin",
  ], {
    CN_API_KEY: "cn_secret_value_12345",
    OPENROUTER_API_KEY: "",
  });

  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /OPENROUTER_API_KEY/);
  assert.match(result.stderr, /preflight failed/i);
});

test("preflight accepts a selected required codex host cell with the existing required secret set", () => {
  const result = run([
    "--suite",
    "provider-auth-matrix-required",
    "--platform",
    "darwin",
    "--cell",
    "codex.endpoint_api_key.local_host",
  ], {
    CN_API_KEY: "cn_secret_value_12345",
    OPENROUTER_API_KEY: "openrouter_secret_value_12345",
  });

  assert.equal(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stdout, /OPENROUTER_API_KEY.*present/i);
  assert.match(result.stdout, /preflight passed/i);
});

test("allow-missing is an explicit local opt out", () => {
  const result = run([
    "--suite",
    "providers-provider-api-auth",
    "--allow-missing",
  ], {
    CTX_E2E_CURSOR_API_KEY: "",
    CTX_E2E_GEMINI_API_KEY: "",
  });

  assert.equal(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /allow-missing enabled/i);
});

test("preflight resolves codex oauth credentials from selected nightly matrix cells", () => {
  const result = run([
    "--suite",
    "provider-auth-matrix-nightly",
    "--platform",
    "darwin",
    "--cell",
    "codex.subscription_oauth.local_host",
  ], {
    CN_API_KEY: "cn_secret_value_12345",
    CTX_E2E_CODEX_OAUTH_EMAIL: "user@example.com",
    CTX_E2E_CODEX_OAUTH_PASSWORD: "",
    CTX_E2E_CODEX_OAUTH_TOTP_SECRET: "JBSWY3DPEHPK3PXP",
  });

  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /CTX_E2E_CODEX_OAUTH_PASSWORD/);
  assert.match(result.stderr, /preflight failed/i);
});

test("preflight trims newline-terminated deferred PATH-backed payloads before validation", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-e2e-deferred-path-"));
  const oauthCredsPath = path.join(tempDir, "gemini-oauth.json");
  fs.writeFileSync(oauthCredsPath, '{"refresh_token":"gemini-refresh-token-12345"}\n', "utf8");

  const result = run([
    "--suite",
    "provider-auth-matrix-nightly",
    "--platform",
    "darwin",
    "--cell",
    "gemini.subscription_oauth.local_host",
    "--include-deferred",
  ], {
    CN_API_KEY: "cn_secret_value_12345",
    CTX_E2E_GEMINI_OAUTH_CREDS_JSON: "",
    CTX_E2E_GEMINI_OAUTH_CREDS_PATH: oauthCredsPath,
  });

  assert.equal(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stdout, /CTX_E2E_GEMINI_OAUTH_CREDS_JSON: present via file:CTX_E2E_GEMINI_OAUTH_CREDS_PATH/i);
  assert.match(result.stdout, /preflight passed/i);
});

test("preflight reports unreadable deferred secret files as invalid instead of crashing", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-e2e-unreadable-path-"));
  const oauthCredsPath = path.join(tempDir, "gemini-oauth.json");
  writeUnreadableFile(oauthCredsPath, '{"refresh_token":"gemini-refresh-token-12345"}\n');

  const result = run([
    "--suite",
    "provider-auth-matrix-nightly",
    "--platform",
    "darwin",
    "--cell",
    "gemini.subscription_oauth.local_host",
    "--include-deferred",
  ], {
    CN_API_KEY: "cn_secret_value_12345",
    CTX_E2E_GEMINI_OAUTH_CREDS_PATH: oauthCredsPath,
  });

  fs.chmodSync(oauthCredsPath, 0o600);
  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /CTX_E2E_GEMINI_OAUTH_CREDS_JSON/);
  assert.match(result.stderr, /file could not be read/i);
  assert.match(result.stderr, /preflight failed/i);
});

test("preflight prefers AMP seed dir over stale JSON env when both are set", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-e2e-amp-home-seed-"));
  const ampHomeSeedDir = path.join(tempDir, "amp-home");
  fs.mkdirSync(ampHomeSeedDir, { recursive: true });

  const result = run([
    "--suite",
    "provider-auth-matrix-nightly",
    "--platform",
    "darwin",
    "--cell",
    "amp.subscription_oauth.local_host",
    "--include-deferred",
  ], {
    CN_API_KEY: "cn_secret_value_12345",
    CTX_E2E_AMP_HOME_SEED_DIR: ampHomeSeedDir,
    CTX_E2E_AMP_SECRETS_JSON: "placeholder",
  });

  assert.equal(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stdout, /CTX_E2E_AMP_SECRETS_JSON: present via dir:CTX_E2E_AMP_HOME_SEED_DIR/i);
  assert.match(result.stdout, /preflight passed/i);
});

test("preflight blocks on an invalid AMP seed dir instead of falling back to lower-priority secrets", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-e2e-amp-invalid-seed-"));
  const invalidAmpSeedPath = path.join(tempDir, "amp-home.txt");
  fs.writeFileSync(invalidAmpSeedPath, "not-a-directory\n", "utf8");

  const result = run([
    "--suite",
    "provider-auth-matrix-nightly",
    "--platform",
    "darwin",
    "--cell",
    "amp.subscription_oauth.local_host",
    "--include-deferred",
  ], {
    CN_API_KEY: "cn_secret_value_12345",
    CTX_E2E_AMP_HOME_SEED_DIR: invalidAmpSeedPath,
    CTX_E2E_AMP_SECRETS_JSON: '{"apiKey@test":"amp-secret-value-12345"}',
  });

  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /CTX_E2E_AMP_SECRETS_JSON/);
  assert.match(result.stderr, /CTX_E2E_AMP_HOME_SEED_DIR is not a directory/i);
  assert.match(result.stderr, /preflight failed/i);
});

test("preflight accepts OpenRouter key from title_generation settings fallback", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-e2e-settings-"));
  const dataRoot = path.join(tempDir, "ctx-data");
  fs.mkdirSync(dataRoot, { recursive: true });
  fs.writeFileSync(path.join(dataRoot, "settings.json"), JSON.stringify({
    title_generation: {
      api_key: "openrouter_secret_value_12345",
    },
  }), "utf8");

  const result = run([
    "--suite",
    "providers-endpoint-ui",
  ], {
    CTX_DATA_ROOT: dataRoot,
    OPENROUTER_API_KEY: "",
  });

  assert.equal(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stdout, /present via settings:title_generation\.api_key/);
  assert.match(result.stdout, /preflight passed/i);
});

test("desktop-remote-real-ci fails when required AWS credentials are missing", () => {
  const result = run([
    "--suite",
    "desktop-remote-real-ci",
  ], {
    CTX_UPDATER_E2E_CLOUD_PROVIDER: "aws",
    AWS_ACCESS_KEY_ID: "",
    AWS_SECRET_ACCESS_KEY: "",
    AWS_REGION: "",
  });

  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /AWS_ACCESS_KEY_ID/);
  assert.match(result.stderr, /AWS_SECRET_ACCESS_KEY/);
  assert.match(result.stderr, /AWS_REGION/);
});

test("desktop-remote-real-ci passes with required AWS credentials present", () => {
  const result = run([
    "--suite",
    "desktop-remote-real-ci",
  ], {
    CTX_UPDATER_E2E_CLOUD_PROVIDER: "aws",
    AWS_ACCESS_KEY_ID: "AKIA1234567890EXAMPLE",
    AWS_SECRET_ACCESS_KEY: "abcdefghijklmnopqrstuvwxyz0123456789ABCD",
    AWS_REGION: "us-east-1",
  });

  assert.equal(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stdout, /preflight passed/i);
});

test("desktop-remote-real-ci local provider passes without cloud credentials", () => {
  const result = run([
    "--suite",
    "desktop-remote-real-ci",
  ], {
    CTX_UPDATER_E2E_CLOUD_PROVIDER: "local",
    AWS_ACCESS_KEY_ID: "",
    AWS_SECRET_ACCESS_KEY: "",
    AWS_REGION: "",
    HETZNER_API_TOKEN: "",
  });

  assert.equal(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stdout, /preflight passed/i);
});
