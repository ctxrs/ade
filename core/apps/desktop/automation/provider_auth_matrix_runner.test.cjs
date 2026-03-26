const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const test = require("node:test");

const repoRoot = path.resolve(__dirname, "..", "..", "..", "..");
const runnerPath = path.join(repoRoot, "core", "apps", "desktop", "scripts", "run_provider_auth_matrix.sh");

const mkTempDir = (prefix) => fs.mkdtempSync(path.join(os.tmpdir(), prefix));

const writeFixture = (dir) => {
  const fixturePath = path.join(dir, "provider_auth_matrix.json");
  const fixture = {
    schema_version: 1,
    generated_at: "2026-03-05",
    summary: "test fixture",
    providers: [{ id: "gemini", owner: "provider-gemini" }],
    auth_modes: [{ id: "subscription_oauth", description: "Managed subscription" }],
    daemon_locations: [{ id: "local", description: "Local daemon" }],
    execution_environments: [{ id: "host", description: "Host execution" }],
    assertion_definitions: {
      probe_success: "ok",
    },
    cells: [
      {
        id: "gemini.subscription_oauth.local.host",
        provider_id: "gemini",
        auth_mode: "subscription_oauth",
        daemon_location: "local",
        execution_environment: "host",
        support: "deferred",
        lane: "nightly",
        required_assertions: ["probe_success"],
        owner: "provider-gemini",
        prerequisites: ["CTX_E2E_GEMINI_OAUTH_CREDS_JSON"],
        runner: {
          kind: "desktop_wdio",
          spec: "automation/specs/provider-auth-matrix-cell.spec.cjs",
          scenarios: "provider,provider-auth-matrix",
        },
        skip_reason: "credential_backed_validation_pending",
      },
    ],
  };
  fs.writeFileSync(fixturePath, `${JSON.stringify(fixture, null, 2)}\n`, "utf8");
  return fixturePath;
};

const runRunner = ({ includeDeferred, insertedSeparator = false, extraEnv = {} }) => {
  const tmp = mkTempDir("ctx-provider-matrix-runner-");
  const artifactsDir = path.join(tmp, "artifacts");
  const fixturePath = writeFixture(tmp);
  const args = ["--lane", "nightly", "--dry-run", "--artifacts-dir", artifactsDir];
  if (includeDeferred) args.push("--include-deferred");
  if (insertedSeparator) args.push("--", "--dry-run");
  const result = spawnSync("bash", [runnerPath, ...args], {
    cwd: repoRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      CTX_PROVIDER_AUTH_MATRIX_FIXTURE: fixturePath,
      ...extraEnv,
    },
  });
  const summaryPath = path.join(artifactsDir, "summary.tsv");
  const summary = fs.existsSync(summaryPath) ? fs.readFileSync(summaryPath, "utf8") : "";
  return {
    ...result,
    summary,
  };
};

const writeGeminiOauthFile = (tmpDir) => {
  const oauthCredsPath = path.join(tmpDir, "gemini-oauth.json");
  fs.writeFileSync(oauthCredsPath, '{"refresh_token":"gemini-refresh-token-12345"}\n', "utf8");
  return oauthCredsPath;
};

test("runner skips deferred cells by default", () => {
  const result = runRunner({ includeDeferred: false });
  assert.equal(result.status, 0, result.stderr || result.stdout);
  assert.match(result.summary, /\tskip\t0\t.*\tdeferred\tdeferred:credential_backed_validation_pending/);
});

test("runner dry-runs deferred cells when include-deferred is set", () => {
  const tmpDir = mkTempDir("ctx-provider-matrix-runner-creds-");
  const oauthCredsPath = writeGeminiOauthFile(tmpDir);
  const result = runRunner({
    includeDeferred: true,
    extraEnv: {
      CTX_E2E_GEMINI_OAUTH_CREDS_PATH: oauthCredsPath,
    },
  });
  assert.equal(result.status, 0, result.stderr || result.stdout);
  assert.match(result.summary, /\tdry-run\t0\t.*\tdeferred\tdry-run/);
});

test("runner tolerates pnpm-style separator before forwarded flags", () => {
  const tmpDir = mkTempDir("ctx-provider-matrix-runner-creds-separator-");
  const oauthCredsPath = writeGeminiOauthFile(tmpDir);
  const result = runRunner({
    includeDeferred: true,
    insertedSeparator: true,
    extraEnv: {
      CTX_E2E_GEMINI_OAUTH_CREDS_PATH: oauthCredsPath,
    },
  });
  assert.equal(result.status, 0, result.stderr || result.stdout);
  assert.match(result.summary, /\tdry-run\t0\t.*\tdeferred\tdry-run/);
});

test("runner enables scoped macOS app sweeps for desktop WDIO cells by default", () => {
  const tmpDir = mkTempDir("ctx-provider-matrix-runner-creds-macos-sweep-");
  const oauthCredsPath = writeGeminiOauthFile(tmpDir);
  const result = runRunner({
    includeDeferred: true,
    extraEnv: {
      CTX_E2E_GEMINI_OAUTH_CREDS_PATH: oauthCredsPath,
    },
  });
  assert.equal(result.status, 0, result.stderr || result.stdout);
  if (process.platform === "darwin") {
    assert.match(result.stdout, /CTX_AUTOMATION_ALLOW_PREP_APP_PROCESS_SWEEP=1/);
  } else {
    assert.doesNotMatch(result.stdout, /CTX_AUTOMATION_ALLOW_PREP_APP_PROCESS_SWEEP=1/);
  }
});
