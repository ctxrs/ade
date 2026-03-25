const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const test = require("node:test");

const repoRoot = path.resolve(__dirname, "..", "..", "..", "..");
const runnerPath = path.join(repoRoot, "core", "apps", "desktop", "scripts", "test_remote_bootstrap_auth_matrix.sh");

const mkTempDir = (prefix) => fs.mkdtempSync(path.join(os.tmpdir(), prefix));

const writeWrapper = (dir, body) => {
  const wrapperPath = path.join(dir, "fake_remote_bootstrap_fixture.sh");
  fs.writeFileSync(wrapperPath, `${body}\n`, { encoding: "utf8", mode: 0o755 });
  return wrapperPath;
};

const runRunner = ({ wrapperBody, cases = "key,password_once" }) => {
  const tmp = mkTempDir("ctx-remote-bootstrap-matrix-runner-");
  const artifactsDir = path.join(tmp, "artifacts");
  fs.mkdirSync(artifactsDir, { recursive: true });
  const wrapperPath = writeWrapper(tmp, wrapperBody);
  const result = spawnSync(
    "bash",
    [runnerPath, "--runtime", "nerdctl", "--cases", cases, "--log-dir", artifactsDir],
    {
      cwd: repoRoot,
      encoding: "utf8",
      env: {
        ...process.env,
        CTX_REMOTE_BOOTSTRAP_FIXTURE_WRAPPER: wrapperPath,
      },
    },
  );
  const summaryPath = path.join(artifactsDir, "remote-bootstrap-auth-matrix-summary.txt");
  return {
    ...result,
    tmp,
    artifactsDir,
    summaryPath,
    summary: fs.existsSync(summaryPath) ? fs.readFileSync(summaryPath, "utf8") : "",
  };
};

test("runner writes summary artifact when the first case fails", () => {
  const result = runRunner({
    wrapperBody: `#!/usr/bin/env bash
set -euo pipefail
case "\${CTX_AUTOMATION_REMOTE_AUTH_TEST_MODE:-key}" in
  key)
    echo "Permission denied" >&2
    exit 1
    ;;
  *)
    echo "ok"
    exit 0
    ;;
esac`,
  });
  try {
    assert.equal(result.status, 1, result.stderr || result.stdout);
    assert.equal(fs.existsSync(result.summaryPath), true);
    assert.match(result.summary, /^remote_bootstrap_auth_matrix/m);
    assert.match(result.summary, /^status=1$/m);
    assert.match(result.summary, /^--- key ---$/m);
    assert.match(result.summary, /^classification=product_or_contract$/m);
  } finally {
    fs.rmSync(result.tmp, { recursive: true, force: true });
  }
});

test("runner summary includes passing cases", () => {
  const result = runRunner({
    wrapperBody: `#!/usr/bin/env bash
set -euo pipefail
echo "fixture ok"
exit 0`,
  });
  try {
    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.equal(fs.existsSync(result.summaryPath), true);
    assert.match(result.summary, /^status=0$/m);
    assert.match(result.summary, /^--- key ---$/m);
    assert.match(result.summary, /^classification=pass$/m);
    assert.match(result.summary, /^--- password_once ---$/m);
  } finally {
    fs.rmSync(result.tmp, { recursive: true, force: true });
  }
});

test("runner assigns fresh driver and backend ports to each case", () => {
  const result = runRunner({
    wrapperBody: `#!/usr/bin/env bash
set -euo pipefail
out_dir="$(cd "$(dirname "$0")" && pwd)"
test_mode=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --test-mode)
      test_mode="\${2:-}"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
cat >"\${out_dir}/\${test_mode}.env" <<EOF
TAURI_DRIVER_PORT=\${TAURI_DRIVER_PORT:-}
TAURI_TEST_BACKEND_PORT=\${TAURI_TEST_BACKEND_PORT:-}
CTX_AUTOMATION_CN_BACKEND_PORT=\${CTX_AUTOMATION_CN_BACKEND_PORT:-}
EOF
exit 0`,
  });
  try {
    assert.equal(result.status, 0, result.stderr || result.stdout);

    const keyEnv = fs.readFileSync(path.join(result.tmp, "key.env"), "utf8");
    const passwordEnv = fs.readFileSync(path.join(result.tmp, "password_once.env"), "utf8");

    const parseEnv = (contents, key) => {
      const match = contents.match(new RegExp(`^${key}=(.+)$`, "m"));
      return match ? match[1] : "";
    };

    const keyDriverPort = parseEnv(keyEnv, "TAURI_DRIVER_PORT");
    const passwordDriverPort = parseEnv(passwordEnv, "TAURI_DRIVER_PORT");
    const keyBackendPort = parseEnv(keyEnv, "TAURI_TEST_BACKEND_PORT");
    const passwordBackendPort = parseEnv(passwordEnv, "TAURI_TEST_BACKEND_PORT");
    const keyCnBackendPort = parseEnv(keyEnv, "CTX_AUTOMATION_CN_BACKEND_PORT");
    const passwordCnBackendPort = parseEnv(passwordEnv, "CTX_AUTOMATION_CN_BACKEND_PORT");

    assert.match(keyDriverPort, /^\d+$/);
    assert.match(passwordDriverPort, /^\d+$/);
    assert.notEqual(keyDriverPort, passwordDriverPort);
    assert.match(keyBackendPort, /^\d+$/);
    assert.match(passwordBackendPort, /^\d+$/);
    assert.notEqual(keyBackendPort, passwordBackendPort);
    assert.match(keyCnBackendPort, /^\d+$/);
    assert.match(passwordCnBackendPort, /^\d+$/);
    assert.notEqual(keyCnBackendPort, passwordCnBackendPort);
  } finally {
    fs.rmSync(result.tmp, { recursive: true, force: true });
  }
});
