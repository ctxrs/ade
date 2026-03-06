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
