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
