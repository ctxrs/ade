const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const childProcess = require("node:child_process");

const scriptPath = path.join(__dirname, "desktop_e2e_redaction_scan.cjs");

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

test("redaction scan fails on a seeded secret assignment leak", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-redaction-fail-"));
  const logPath = path.join(tempDir, "run.log");
  fs.writeFileSync(logPath, "OPENROUTER_API_KEY=openrouter_secret_value_12345\n", "utf8");

  const result = run([
    "--suite",
    "providers-endpoint-ui",
    "--path",
    logPath,
  ]);

  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /OPENROUTER_API_KEY/);
  assert.match(result.stderr, /redaction scan failed/i);
});

test("redaction scan honors allowlist rows with justification", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-redaction-pass-"));
  const logPath = path.join(tempDir, "run.log");
  const allowlistPath = path.join(tempDir, "allowlist.tsv");
  fs.writeFileSync(logPath, "OPENROUTER_API_KEY=openrouter_secret_value_12345\n", "utf8");
  fs.writeFileSync(
    allowlistPath,
    "run\\.log\tOPENROUTER_API_KEY=openrouter_secret_value_12345\tfixture false positive for scanner coverage\n",
    "utf8",
  );

  const result = run([
    "--suite",
    "providers-endpoint-ui",
    "--path",
    logPath,
    "--allowlist",
    allowlistPath,
  ]);

  assert.equal(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stdout, /redaction scan passed/i);
});

test("redaction scan fails on file-backed deferred provider-auth secrets during include-deferred runs", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-redaction-deferred-file-"));
  const oauthCredsPath = path.join(tempDir, "gemini-oauth.json");
  const leakPath = path.join(tempDir, "run.log");
  const oauthCreds = '{"refresh_token":"gemini-refresh-token-12345"}';
  fs.writeFileSync(oauthCredsPath, `${oauthCreds}\n`, "utf8");
  fs.writeFileSync(leakPath, `provider_auth_leak=${oauthCreds}\n`, "utf8");

  const result = run([
    "--suite",
    "provider-auth-matrix-nightly",
    "--platform",
    "darwin",
    "--cell",
    "gemini.subscription_oauth.local_host",
    "--include-deferred",
    "--path",
    leakPath,
  ], {
    CN_API_KEY: "cn_secret_value_12345",
    CTX_E2E_GEMINI_OAUTH_CREDS_JSON: "",
    CTX_E2E_GEMINI_OAUTH_CREDS_PATH: oauthCredsPath,
  });

  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /CTX_E2E_GEMINI_OAUTH_CREDS_JSON/);
  assert.match(result.stderr, /literal-secret/);
});

test("redaction scan fails on Amp seed-dir secret leaks during include-deferred runs", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-redaction-amp-seed-"));
  const ampHomeSeedDir = path.join(tempDir, "amp-home");
  const secretsDir = path.join(ampHomeSeedDir, ".local", "share", "amp");
  const leakPath = path.join(tempDir, "run.log");
  const ampSecrets = '{"apiKey@test":"amp-secret-value-12345"}';
  fs.mkdirSync(secretsDir, { recursive: true });
  fs.writeFileSync(path.join(secretsDir, "secrets.json"), `${ampSecrets}\n`, "utf8");
  fs.writeFileSync(leakPath, `amp_leak=${ampSecrets}\n`, "utf8");

  const result = run([
    "--suite",
    "provider-auth-matrix-nightly",
    "--platform",
    "darwin",
    "--cell",
    "amp.subscription_oauth.local_host",
    "--include-deferred",
    "--path",
    leakPath,
  ], {
    CN_API_KEY: "cn_secret_value_12345",
    CTX_E2E_AMP_HOME_SEED_DIR: ampHomeSeedDir,
  });

  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /CTX_E2E_AMP_SECRETS_JSON/);
  assert.match(result.stderr, /literal-secret/);
});

test("redaction scan fails on Mistral seed-dir secret leaks during include-deferred runs", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-redaction-mistral-seed-"));
  const mistralHomeSeedDir = path.join(tempDir, "mistral-home");
  const leakPath = path.join(tempDir, "run.log");
  const mistralSecrets = '{"refresh_token":"mistral-refresh-token-12345"}';
  fs.mkdirSync(mistralHomeSeedDir, { recursive: true });
  fs.writeFileSync(path.join(mistralHomeSeedDir, "auth.json"), `${mistralSecrets}\n`, "utf8");
  fs.writeFileSync(leakPath, `mistral_leak=${mistralSecrets}\n`, "utf8");

  const result = run([
    "--suite",
    "provider-auth-matrix-nightly",
    "--platform",
    "darwin",
    "--cell",
    "mistral.subscription_oauth.local_host",
    "--include-deferred",
    "--path",
    leakPath,
  ], {
    CN_API_KEY: "cn_secret_value_12345",
    CTX_E2E_MISTRAL_HOME_SEED_DIR: mistralHomeSeedDir,
  });

  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /CTX_E2E_MISTRAL_HOME_SEED_DIR/);
  assert.match(result.stderr, /literal-secret/);
});

test("redaction scan reports unreadable deferred secret files as invalid instead of crashing", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-redaction-unreadable-file-"));
  const oauthCredsPath = path.join(tempDir, "gemini-oauth.json");
  const logPath = path.join(tempDir, "run.log");
  writeUnreadableFile(oauthCredsPath, '{"refresh_token":"gemini-refresh-token-12345"}\n');
  fs.writeFileSync(logPath, "no leak here\n", "utf8");

  const result = run([
    "--suite",
    "provider-auth-matrix-nightly",
    "--platform",
    "darwin",
    "--cell",
    "gemini.subscription_oauth.local_host",
    "--include-deferred",
    "--path",
    logPath,
  ], {
    CN_API_KEY: "cn_secret_value_12345",
    CTX_E2E_GEMINI_OAUTH_CREDS_PATH: oauthCredsPath,
  });

  fs.chmodSync(oauthCredsPath, 0o600);
  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /redaction scan invalid/i);
  assert.match(result.stderr, /CTX_E2E_GEMINI_OAUTH_CREDS_JSON/);
  assert.match(result.stderr, /file could not be read/i);
});
