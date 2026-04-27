const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const childProcess = require("node:child_process");

const scriptPath = path.join(__dirname, "run_provider_auth_matrix.sh");

const testTmpRoot = () => {
  const root = path.join(os.homedir(), ".ctx", "volatile", "tmp");
  fs.mkdirSync(root, { recursive: true });
  return root;
};

const writeExecutable = (filePath, contents) => {
  fs.writeFileSync(filePath, contents, { encoding: "utf8", mode: 0o755 });
};

const createFixture = (
  tmpDir,
  {
    id = "codex.endpoint_api_key.local.sandbox",
    executionEnvironment = "sandbox",
    scenarios = "local-codex-smoke",
  } = {},
) => {
  const fixturePath = path.join(tmpDir, "provider_auth_matrix.json");
  fs.writeFileSync(
    fixturePath,
    JSON.stringify(
      {
        cells: [
          {
            id,
            provider_id: "codex",
            auth_mode: "endpoint_api_key",
            daemon_location: "local",
            execution_environment: executionEnvironment,
            support: "supported",
            lane: "required",
            prerequisites: ["OPENROUTER_API_KEY"],
            runner: {
              kind: "desktop_wdio",
              spec: "automation/specs/provider-auth-matrix-cell.spec.cjs",
              scenarios,
            },
          },
        ],
      },
      null,
      2,
    ),
    "utf8",
  );
  return fixturePath;
};

const createDeferredGeminiFixture = (tmpDir) => {
  const fixturePath = path.join(tmpDir, "provider_auth_matrix.json");
  fs.writeFileSync(
    fixturePath,
    JSON.stringify(
      {
        cells: [
          {
            id: "gemini.subscription_oauth.local.host",
            provider_id: "gemini",
            auth_mode: "subscription_oauth",
            daemon_location: "local",
            execution_environment: "host",
            support: "deferred",
            lane: "nightly",
            prerequisites: ["CTX_E2E_GEMINI_OAUTH_CREDS_JSON"],
            skip_reason: "missing_ci_secret_contract",
            runner: {
              kind: "desktop_wdio",
              spec: "automation/specs/provider-auth-matrix-cell.spec.cjs",
              scenarios: "provider,provider-auth-matrix",
            },
          },
        ],
      },
      null,
      2,
    ),
    "utf8",
  );
  return fixturePath;
};

const createPreflightScript = (tmpDir) => {
  const preflightPath = path.join(tmpDir, "fake_preflight.cjs");
  writeExecutable(
    preflightPath,
    `#!/usr/bin/env node
const fs = require("node:fs");
const path = require("node:path");
const out = process.env.CTX_TEST_PREFLIGHT_OUT;
if (out) fs.writeFileSync(out, process.env.OPENROUTER_API_KEY ? "set\\n" : "unset\\n", "utf8");
if (!process.env.OPENROUTER_API_KEY) {
  console.error("OPENROUTER_API_KEY missing");
  process.exit(1);
}
console.log("preflight passed");
`,
  );
  return preflightPath;
};

const createPassthroughPreflightScript = (tmpDir) => {
  const preflightPath = path.join(tmpDir, "fake_preflight_pass.cjs");
  writeExecutable(
    preflightPath,
    `#!/usr/bin/env node
console.log("preflight passed");
`,
  );
  return preflightPath;
};

const createSmokeScript = (tmpDir) => {
  const smokePath = path.join(tmpDir, "fake_smoke.sh");
  writeExecutable(
    smokePath,
    `#!/bin/bash
set -euo pipefail
printf '%s\\n' "$([[ -n "\${OPENROUTER_API_KEY:-}" ]] && echo set || echo unset)" >"\${CTX_TEST_SMOKE_OUT:?}"
if [[ -n "\${CTX_TEST_ENV_OUT:-}" ]]; then
  {
    printf 'CTX_BUNDLE_REMOTE_DAEMONS=%s\\n' "\${CTX_BUNDLE_REMOTE_DAEMONS:-}"
    printf 'CTX_BUNDLE_LINUX_CTX_MCP_RUNTIME=%s\\n' "\${CTX_BUNDLE_LINUX_CTX_MCP_RUNTIME:-}"
    printf 'CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD=%s\\n' "\${CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD:-}"
    printf 'CTX_AUTOMATION_KEEP_TMPDIR=%s\\n' "\${CTX_AUTOMATION_KEEP_TMPDIR:-}"
  } >"\${CTX_TEST_ENV_OUT}"
fi
if [[ -n "\${CTX_TEST_DAEMON_DATA_ROOT:-}" ]]; then
  mkdir -p "\${CTX_TEST_DAEMON_DATA_ROOT}/logs/providers"
  mkdir -p "\${CTX_TEST_DAEMON_DATA_ROOT}/managed/vms/avf-linux/shared/logs"
  mkdir -p "\${CTX_TEST_DAEMON_DATA_ROOT}/managed/vms/avf-linux/shared/worktrees/wt/shadow-root/.git/logs"
  printf 'stderr\\n' >"\${CTX_TEST_DAEMON_DATA_ROOT}/logs/providers/crp-codex.stderr.log"
  printf 'vm\\n' >"\${CTX_TEST_DAEMON_DATA_ROOT}/managed/vms/avf-linux/shared/logs/shared-vm.log"
  printf 'git\\n' >"\${CTX_TEST_DAEMON_DATA_ROOT}/managed/vms/avf-linux/shared/worktrees/wt/shadow-root/.git/logs/HEAD"
  printf '{"state":"Running"}\\n' >"\${CTX_TEST_DAEMON_DATA_ROOT}/managed/vms/avf-linux/shared/shared-vm-state.json"
  cat >"\${CTX_PROVIDER_AUTH_MATRIX_REPORT:?}" <<JSON
{"result":"fail","reason":"cell failed","artifacts":{"daemon_diagnostics_on_failure":{"daemon":{"data_root":"\${CTX_TEST_DAEMON_DATA_ROOT}"}}}}
JSON
  exit 0
fi
cat >"\${CTX_PROVIDER_AUTH_MATRIX_REPORT:?}" <<'JSON'
{"result":"pass","reason":"ok"}
JSON
	`,
  );
  return smokePath;
};

const createInfisicalScript = (tmpDir) => {
  const binDir = path.join(tmpDir, "bin");
  fs.mkdirSync(binDir, { recursive: true });
  const infisicalPath = path.join(binDir, "infisical");
  writeExecutable(
    infisicalPath,
    `#!/bin/bash
set -euo pipefail
while [[ "$#" -gt 0 ]]; do
  if [[ "$1" == "--" ]]; then
    shift
    break
  fi
  case "$1" in
    --env|--projectId)
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
exec env OPENROUTER_API_KEY="injected-openrouter-key" CN_API_KEY="injected-cn-key" "$@"
`,
  );
  return { binDir, infisicalPath };
};

const runMatrixScript = ({
  fixturePath,
  preflightPath,
  smokePath,
  pathPrefix,
  infisicalConfigPath,
  preflightOut,
  smokeOut,
  artifactsDir,
  lane = "required",
  cellId = "codex.endpoint_api_key.local.sandbox",
  extraArgs = [],
  extraEnv = {},
}) =>
  childProcess.spawnSync("bash", [
    scriptPath,
    "--lane",
    lane,
    "--cell",
    cellId,
    "--artifacts-dir",
    artifactsDir,
    ...extraArgs,
  ], {
    cwd: path.join(__dirname, "..", "..", "..", ".."),
    encoding: "utf8",
    env: {
      ...process.env,
      PATH: `${pathPrefix}${path.delimiter}${process.env.PATH}`,
      CTX_PROVIDER_AUTH_MATRIX_FIXTURE: fixturePath,
      CTX_PROVIDER_AUTH_MATRIX_PREFLIGHT_SCRIPT: preflightPath,
      CTX_PROVIDER_AUTH_MATRIX_SMOKE_SCRIPT: smokePath,
      CTX_PROVIDER_AUTH_MATRIX_INFISICAL_CONFIG_FILE: infisicalConfigPath,
      CTX_TEST_PREFLIGHT_OUT: preflightOut,
      CTX_TEST_SMOKE_OUT: smokeOut,
      OPENROUTER_API_KEY: "",
      CN_API_KEY: "",
      ...extraEnv,
    },
  });

test("run_provider_auth_matrix hydrates preflight and runner through infisical when available", () => {
  const tmpDir = fs.mkdtempSync(path.join(testTmpRoot(), "ctx-run-provider-auth-matrix-"));
  const fixturePath = createFixture(tmpDir);
  const preflightPath = createPreflightScript(tmpDir);
  const smokePath = createSmokeScript(tmpDir);
  const { binDir } = createInfisicalScript(tmpDir);
  const infisicalConfigPath = path.join(tmpDir, ".infisical.json");
  fs.writeFileSync(infisicalConfigPath, "{}\n", "utf8");
  const preflightOut = path.join(tmpDir, "preflight.out");
  const smokeOut = path.join(tmpDir, "smoke.out");
  const artifactsDir = path.join(tmpDir, "artifacts");

  const result = runMatrixScript({
    fixturePath,
    preflightPath,
    smokePath,
    pathPrefix: binDir,
    infisicalConfigPath,
    preflightOut,
    smokeOut,
    artifactsDir,
  });

  assert.equal(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.equal(fs.readFileSync(preflightOut, "utf8").trim(), "set");
  assert.equal(fs.readFileSync(smokeOut, "utf8").trim(), "set");
  const summary = fs.readFileSync(path.join(artifactsDir, "summary.tsv"), "utf8");
  assert.match(summary, /codex\.endpoint_api_key\.local\.sandbox\tpass\t0\t/);
});

test("run_provider_auth_matrix scopes local host desktop prep away from remote daemons and AVF payloads", () => {
  const tmpDir = fs.mkdtempSync(path.join(testTmpRoot(), "ctx-run-provider-auth-matrix-host-"));
  const fixturePath = createFixture(tmpDir, {
    id: "codex.endpoint_api_key.local.host",
    executionEnvironment: "host",
    scenarios: "local-codex-host-smoke",
  });
  const preflightPath = createPassthroughPreflightScript(tmpDir);
  const smokePath = createSmokeScript(tmpDir);
  const artifactsDir = path.join(tmpDir, "artifacts");
  const envOut = path.join(tmpDir, "env.out");

  const result = runMatrixScript({
    fixturePath,
    preflightPath,
    smokePath,
    pathPrefix: process.env.PATH || "",
    infisicalConfigPath: path.join(tmpDir, "missing-infisical.json"),
    preflightOut: path.join(tmpDir, "preflight.out"),
    smokeOut: path.join(tmpDir, "smoke.out"),
    artifactsDir,
    cellId: "codex.endpoint_api_key.local.host",
    extraEnv: {
      CTX_PROVIDER_AUTH_MATRIX_USE_INFISICAL: "0",
      OPENROUTER_API_KEY: "test-openrouter-key",
      CN_API_KEY: "test-cn-key",
      CTX_TEST_ENV_OUT: envOut,
    },
  });

  assert.equal(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  const envText = fs.readFileSync(envOut, "utf8");
  assert.match(envText, /^CTX_BUNDLE_REMOTE_DAEMONS=0$/m);
  assert.match(envText, /^CTX_BUNDLE_LINUX_CTX_MCP_RUNTIME=0$/m);
  assert.match(envText, /^CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD=1$/m);
  assert.match(envText, /^CTX_AUTOMATION_KEEP_TMPDIR=1$/m);
});

test("run_provider_auth_matrix keeps Linux MCP runtime available for local sandbox cells", () => {
  const tmpDir = fs.mkdtempSync(path.join(testTmpRoot(), "ctx-run-provider-auth-matrix-sandbox-mcp-"));
  const fixturePath = createFixture(tmpDir, {
    id: "codex.endpoint_api_key.local.sandbox",
    executionEnvironment: "sandbox",
    scenarios: "local-codex-sandbox-smoke",
  });
  const preflightPath = createPassthroughPreflightScript(tmpDir);
  const smokePath = createSmokeScript(tmpDir);
  const artifactsDir = path.join(tmpDir, "artifacts");
  const envOut = path.join(tmpDir, "env.out");

  const result = runMatrixScript({
    fixturePath,
    preflightPath,
    smokePath,
    pathPrefix: process.env.PATH || "",
    infisicalConfigPath: path.join(tmpDir, "missing-infisical.json"),
    preflightOut: path.join(tmpDir, "preflight.out"),
    smokeOut: path.join(tmpDir, "smoke.out"),
    artifactsDir,
    cellId: "codex.endpoint_api_key.local.sandbox",
    extraEnv: {
      CTX_PROVIDER_AUTH_MATRIX_USE_INFISICAL: "0",
      OPENROUTER_API_KEY: "test-openrouter-key",
      CN_API_KEY: "test-cn-key",
      CTX_TEST_ENV_OUT: envOut,
    },
  });

  assert.equal(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  const envText = fs.readFileSync(envOut, "utf8");
  assert.match(envText, /^CTX_BUNDLE_REMOTE_DAEMONS=0$/m);
  assert.match(envText, /^CTX_BUNDLE_LINUX_CTX_MCP_RUNTIME=$/m);
  assert.match(envText, /^CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD=$/m);
});

test("run_provider_auth_matrix leaves preflight strict when infisical auto-hydration is disabled", () => {
  const tmpDir = fs.mkdtempSync(path.join(testTmpRoot(), "ctx-run-provider-auth-matrix-no-infisical-"));
  const fixturePath = createFixture(tmpDir);
  const preflightPath = createPreflightScript(tmpDir);
  const smokePath = createSmokeScript(tmpDir);
  const { binDir } = createInfisicalScript(tmpDir);
  const infisicalConfigPath = path.join(tmpDir, ".infisical.json");
  fs.writeFileSync(infisicalConfigPath, "{}\n", "utf8");
  const preflightOut = path.join(tmpDir, "preflight.out");
  const smokeOut = path.join(tmpDir, "smoke.out");
  const artifactsDir = path.join(tmpDir, "artifacts");

  const result = runMatrixScript({
    fixturePath,
    preflightPath,
    smokePath,
    pathPrefix: binDir,
    infisicalConfigPath,
    preflightOut,
    smokeOut,
    artifactsDir,
    extraEnv: {
      CTX_PROVIDER_AUTH_MATRIX_USE_INFISICAL: "0",
    },
  });

  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.equal(fs.readFileSync(preflightOut, "utf8").trim(), "unset");
  assert.equal(fs.existsSync(smokeOut), false);
});

test("run_provider_auth_matrix copies daemon diagnostics from failed cell reports", () => {
  const tmpDir = fs.mkdtempSync(path.join(testTmpRoot(), "ctx-run-provider-auth-matrix-diag-"));
  const fixturePath = createFixture(tmpDir);
  const preflightPath = createPassthroughPreflightScript(tmpDir);
  const smokePath = createSmokeScript(tmpDir);
  const artifactsDir = path.join(tmpDir, "artifacts");
  const daemonDataRoot = path.join(tmpDir, "daemon-data");

  const result = runMatrixScript({
    fixturePath,
    preflightPath,
    smokePath,
    pathPrefix: process.env.PATH || "",
    infisicalConfigPath: path.join(tmpDir, "missing-infisical.json"),
    preflightOut: path.join(tmpDir, "preflight.out"),
    smokeOut: path.join(tmpDir, "smoke.out"),
    artifactsDir,
    extraEnv: {
      CTX_PROVIDER_AUTH_MATRIX_USE_INFISICAL: "0",
      CTX_PROVIDER_AUTH_MATRIX_RETRY_LIMIT: "0",
      OPENROUTER_API_KEY: "test-openrouter-key",
      CN_API_KEY: "test-cn-key",
      CTX_TEST_DAEMON_DATA_ROOT: daemonDataRoot,
    },
  });

  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  const cellDir = path.join(artifactsDir, "codex.endpoint_api_key.local.sandbox");
  assert.equal(
    fs.readFileSync(path.join(cellDir, "diagnostics", "attempt-1", "daemon-logs", "providers", "crp-codex.stderr.log"), "utf8"),
    "stderr\n",
  );
  assert.equal(
    fs.readFileSync(path.join(cellDir, "diagnostics", "attempt-1", "avf", "avf-linux", "shared", "logs", "shared-vm.log"), "utf8"),
    "vm\n",
  );
  assert.equal(
    fs.readFileSync(path.join(cellDir, "diagnostics", "attempt-1", "avf", "avf-linux", "shared", "shared-vm-state.json"), "utf8"),
    '{"state":"Running"}\n',
  );
  assert.equal(
    fs.existsSync(path.join(cellDir, "diagnostics", "attempt-1", "avf", "avf-linux", "shared", "worktrees", "wt", "shadow-root", ".git", "logs", "HEAD")),
    false,
  );
});
test("run_provider_auth_matrix accepts file-backed deferred oauth prerequisites during dry-run selection", () => {
  const tmpDir = fs.mkdtempSync(path.join(testTmpRoot(), "ctx-run-provider-auth-matrix-deferred-"));
  const fixturePath = createDeferredGeminiFixture(tmpDir);
  const preflightPath = createPassthroughPreflightScript(tmpDir);
  const smokePath = createSmokeScript(tmpDir);
  const oauthCredsPath = path.join(tmpDir, "gemini-oauth.json");
  fs.writeFileSync(oauthCredsPath, '{"refresh_token":"gemini-refresh-token-12345"}\n', "utf8");
  const artifactsDir = path.join(tmpDir, "artifacts");

  const result = runMatrixScript({
    fixturePath,
    preflightPath,
    smokePath,
    pathPrefix: process.env.PATH || "",
    infisicalConfigPath: path.join(tmpDir, "missing-infisical.json"),
    preflightOut: path.join(tmpDir, "preflight.out"),
    smokeOut: path.join(tmpDir, "smoke.out"),
    artifactsDir,
    lane: "nightly",
    cellId: "gemini.subscription_oauth.local.host",
    extraArgs: ["--include-deferred", "--dry-run"],
    extraEnv: {
      CTX_PROVIDER_AUTH_MATRIX_USE_INFISICAL: "0",
      CN_API_KEY: "cn_secret_value_12345",
      CTX_E2E_GEMINI_OAUTH_CREDS_JSON: "",
      CTX_E2E_GEMINI_OAUTH_CREDS_PATH: oauthCredsPath,
    },
  });

  assert.equal(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  const summary = fs.readFileSync(path.join(artifactsDir, "summary.tsv"), "utf8");
  assert.match(summary, /gemini\.subscription_oauth\.local\.host\tdry-run\t0\t/);
  assert.doesNotMatch(summary, /missing_env:/);
});

test("run_provider_auth_matrix retry classification uses portable grep", () => {
  const text = fs.readFileSync(scriptPath, "utf8");

  assert.doesNotMatch(text, /\brg -q\b/);
  assert.match(text, /grep -E -q[\s\S]*Request failed with error code ECONNREFUSED/s);
  assert.match(text, /grep -E -q[\s\S]*codex oauth login did not succeed/s);
});
