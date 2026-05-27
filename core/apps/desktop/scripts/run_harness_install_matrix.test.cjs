#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");
const childProcess = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const scriptPath = path.join(__dirname, "run_harness_install_matrix.sh");
const repoRoot = path.resolve(__dirname, "..", "..", "..", "..");

const testTmpRoot = () => {
  const root = path.join(os.homedir(), ".ctx", "volatile", "tmp");
  fs.mkdirSync(root, { recursive: true });
  return root;
};

const writeExecutable = (filePath, contents) => {
  fs.writeFileSync(filePath, contents, { encoding: "utf8", mode: 0o755 });
};

const writeFixture = (tmpDir) => {
  const fixturePath = path.join(tmpDir, "harness_install_matrix.json");
  const manifest = {
    schema_version: 1,
    generated_at: "2026-04-29",
    platform_targets: [
      { id: "macos.host", platform: "macos", execution_target: "host", install_target: "host" },
      { id: "macos.sandbox", platform: "macos", execution_target: "sandbox", install_target: "container" },
      { id: "linux.host", platform: "linux", execution_target: "host", install_target: "host" },
      { id: "linux.sandbox", platform: "linux", execution_target: "sandbox", install_target: "container" },
    ],
    providers: [
      {
        id: "codex",
        display_name: "Codex",
        install_kind: "archive",
        cells: {
          "macos.host": cell(["preview", "release", "nightly"], "host"),
          "macos.sandbox": cell(["preview", "release", "nightly"], "container"),
          "linux.host": cell(["release", "nightly"], "host"),
          "linux.sandbox": cell(["release", "nightly"], "container"),
        },
      },
      {
        id: "gemini",
        display_name: "Gemini",
        install_kind: "npm",
        cells: {
          "macos.host": cell(["preview", "release", "nightly"], "host"),
          "macos.sandbox": cell(["preview", "release", "nightly"], "container"),
          "linux.host": cell(["release", "nightly"], "host"),
          "linux.sandbox": cell(["release", "nightly"], "container"),
        },
      },
    ],
  };
  fs.writeFileSync(fixturePath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  return fixturePath;
};

const writeReleasePlan = (tmpDir) => {
  const sourceCommit = childProcess.execFileSync("git", ["-C", repoRoot, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
  const releasePlanPath = path.join(tmpDir, "resolved_release_plan.json");
  fs.writeFileSync(releasePlanPath, `${JSON.stringify({
    schema_version: 1,
    generated_at: "2026-05-26T00:00:00.000Z",
    kind: "resolved_release_plan",
    source_commit: sourceCommit,
    release_version: "1.2.3",
    channel: "stable",
    storage_channel: "stable",
    release_scope: "auto",
    provider_manifest: {
      digest_sha256: "b".repeat(64),
      id: "provider-manifest-stable",
      required_provider_ids: ["codex"],
      source_commit: sourceCommit,
      url: "https://example.invalid/provider-manifest.json",
    },
    inventory: {
      provider_artifact_policy: "required",
      required_provider_ids: ["codex"],
      missing_provider_ids: [],
      empty_target_provider_ids: [],
      required_targets: [{
        provider_id: "codex",
        target_key: "linux-x64",
        url: "https://example.invalid/codex.tar.zst",
        sha256: "a".repeat(64),
      }],
    },
  }, null, 2)}\n`, "utf8");
  return releasePlanPath;
};

const writeMismatchedReleasePlan = (tmpDir) => {
  const releasePlanPath = writeReleasePlan(tmpDir);
  const plan = JSON.parse(fs.readFileSync(releasePlanPath, "utf8"));
  plan.source_commit = "f".repeat(40);
  plan.provider_manifest.source_commit = "f".repeat(40);
  fs.writeFileSync(releasePlanPath, `${JSON.stringify(plan, null, 2)}\n`, "utf8");
  return releasePlanPath;
};

const cell = (lanes, installTarget) => ({
  support: "supported",
  lanes,
  install_target: installTarget,
  runner: {
    kind: "release_runtime_install_smoke",
    script: "scripts/release_runtime_install_smoke.sh",
  },
});

const writeSmokeScript = (tmpDir) => {
  const smokePath = path.join(tmpDir, "fake_release_runtime_install_smoke.sh");
  writeExecutable(
    smokePath,
    `#!/usr/bin/env bash
set -euo pipefail
printf '%s\\n' "$*" >>"\${CTX_TEST_SMOKE_ARGS:?}"
provider=""
while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --provider)
      provider="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
if [[ "$provider" == "gemini" ]]; then
  echo "forced gemini failure" >&2
  exit 42
fi
`,
  );
  return smokePath;
};

const runScript = ({ fixturePath, smokePath, artifactsDir, extraArgs = [], extraEnv = {} }) =>
  childProcess.spawnSync("bash", [scriptPath, ...extraArgs], {
    cwd: repoRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      CTX_HARNESS_INSTALL_MATRIX_FIXTURE: fixturePath,
      CTX_HARNESS_INSTALL_MATRIX_SMOKE_SCRIPT: smokePath,
      CTX_HARNESS_INSTALL_MATRIX_DAEMON_BIN: "/bin/echo",
      CTX_HARNESS_INSTALL_MATRIX_BUNDLE_DIR: "/tmp/ctx-bundles-test",
      CTX_HARNESS_INSTALL_MATRIX_ARTIFACTS_DIR: artifactsDir,
      CTX_HARNESS_INSTALL_MATRIX_TIMEOUT_SECONDS: "5",
      ...extraEnv,
    },
  });

test("run_harness_install_matrix lists selected release cells", () => {
  const tmpDir = fs.mkdtempSync(path.join(testTmpRoot(), "ctx-harness-install-matrix-"));
  const fixturePath = writeFixture(tmpDir);
  const smokePath = writeSmokeScript(tmpDir);
  const result = runScript({
    fixturePath,
    smokePath,
    artifactsDir: path.join(tmpDir, "artifacts"),
    extraArgs: ["--lane", "release", "--platform", "linux", "--target", "sandbox", "--list"],
  });

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /codex\.linux\.sandbox/);
  assert.match(result.stdout, /gemini\.linux\.sandbox/);
  assert.doesNotMatch(result.stdout, /macos/);
});

test("run_harness_install_matrix dry-run prints exact provider and install target", () => {
  const tmpDir = fs.mkdtempSync(path.join(testTmpRoot(), "ctx-harness-install-matrix-"));
  const fixturePath = writeFixture(tmpDir);
  const smokePath = writeSmokeScript(tmpDir);
  const result = runScript({
    fixturePath,
    smokePath,
    artifactsDir: path.join(tmpDir, "artifacts"),
    extraArgs: [
      "--lane",
      "release",
      "--platform",
      "macos",
      "--target",
      "sandbox",
      "--provider",
      "codex",
      "--dry-run",
    ],
  });

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /dry-run codex\.macos\.sandbox:/);
  assert.match(result.stdout, /--provider codex/);
  assert.match(result.stdout, /--target container/);
});

test("run_harness_install_matrix rejects unmatched requested providers", () => {
  const tmpDir = fs.mkdtempSync(path.join(testTmpRoot(), "ctx-harness-install-matrix-"));
  const fixturePath = writeFixture(tmpDir);
  const smokePath = writeSmokeScript(tmpDir);
  const result = runScript({
    fixturePath,
    smokePath,
    artifactsDir: path.join(tmpDir, "artifacts"),
    extraArgs: [
      "--lane",
      "release",
      "--platform",
      "linux",
      "--target",
      "host",
      "--provider",
      "codex,gemnii",
      "--dry-run",
    ],
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /requested provider selected no supported cells after filters: gemnii/);
  assert.doesNotMatch(result.stdout, /dry-run codex\.linux\.host/);
});

test("run_harness_install_matrix rejects unmatched requested cells", () => {
  const tmpDir = fs.mkdtempSync(path.join(testTmpRoot(), "ctx-harness-install-matrix-"));
  const fixturePath = writeFixture(tmpDir);
  const smokePath = writeSmokeScript(tmpDir);
  const result = runScript({
    fixturePath,
    smokePath,
    artifactsDir: path.join(tmpDir, "artifacts"),
    extraArgs: [
      "--lane",
      "release",
      "--platform",
      "linux",
      "--cell",
      "codex.linux.host,codex.linux.missing",
      "--dry-run",
    ],
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /requested cell selected no supported cells after filters: codex\.linux\.missing/);
  assert.doesNotMatch(result.stdout, /dry-run codex\.linux\.host/);
});

test("run_harness_install_matrix continues through failures and reports all selected cells", () => {
  const tmpDir = fs.mkdtempSync(path.join(testTmpRoot(), "ctx-harness-install-matrix-"));
  const fixturePath = writeFixture(tmpDir);
  const smokePath = writeSmokeScript(tmpDir);
  const smokeArgs = path.join(tmpDir, "smoke-args.log");
  const artifactsDir = path.join(tmpDir, "artifacts");
  const result = runScript({
    fixturePath,
    smokePath,
    artifactsDir,
    extraArgs: ["--lane", "release", "--platform", "linux", "--target", "host"],
    extraEnv: { CTX_TEST_SMOKE_ARGS: smokeArgs },
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /failed 1 cell/);
  const argsLog = fs.readFileSync(smokeArgs, "utf8");
  assert.match(argsLog, /--provider codex/);
  assert.match(argsLog, /--provider gemini/);
  const summary = fs.readFileSync(path.join(artifactsDir, "summary.jsonl"), "utf8");
  assert.match(summary, /"provider_id":"codex"/);
  assert.match(summary, /"provider_id":"gemini"/);
  assert.match(summary, /"status":"failed"/);
  const evidence = JSON.parse(fs.readFileSync(path.join(artifactsDir, "harness-install-evidence.json"), "utf8"));
  assert.equal(evidence.kind, "ctx.harness_install_matrix_evidence.v1");
  assert.equal(evidence.selection.platform, "linux");
  assert.deepEqual(evidence.failed_cells, ["gemini.linux.host"]);
  assert.ok(evidence.host.arch);
});

test("run_harness_install_matrix binds evidence to resolved release plan identity", () => {
  const tmpDir = fs.mkdtempSync(path.join(testTmpRoot(), "ctx-harness-install-matrix-"));
  const fixturePath = writeFixture(tmpDir);
  const smokePath = writeSmokeScript(tmpDir);
  const releasePlanPath = writeReleasePlan(tmpDir);
  const artifactsDir = path.join(tmpDir, "artifacts");
  const result = runScript({
    fixturePath,
    smokePath,
    artifactsDir,
    extraArgs: ["--lane", "release", "--platform", "linux", "--target", "host", "--provider", "codex", "--dry-run"],
    extraEnv: {
      CTX_HARNESS_INSTALL_MATRIX_RELEASE_PLAN: releasePlanPath,
    },
  });

  assert.equal(result.status, 0, result.stderr);
  const evidence = JSON.parse(fs.readFileSync(path.join(artifactsDir, "harness-install-evidence.json"), "utf8"));
  assert.match(evidence.release_plan_digest, /^[a-f0-9]{64}$/u);
  assert.equal(evidence.release_plan_id, `release-plan-sha256:${evidence.release_plan_digest}`);
});

test("run_harness_install_matrix rejects release plan source mismatch before running cells", () => {
  const tmpDir = fs.mkdtempSync(path.join(testTmpRoot(), "ctx-harness-install-matrix-"));
  const fixturePath = writeFixture(tmpDir);
  const smokePath = writeSmokeScript(tmpDir);
  const releasePlanPath = writeMismatchedReleasePlan(tmpDir);
  const smokeArgs = path.join(tmpDir, "smoke-args.log");
  const result = runScript({
    fixturePath,
    smokePath,
    artifactsDir: path.join(tmpDir, "artifacts"),
    extraArgs: ["--lane", "release", "--platform", "linux", "--target", "host"],
    extraEnv: {
      CTX_HARNESS_INSTALL_MATRIX_RELEASE_PLAN: releasePlanPath,
      CTX_TEST_SMOKE_ARGS: smokeArgs,
    },
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /resolved release plan source_commit mismatch/);
  assert.equal(fs.existsSync(smokeArgs), false);
});

test("run_harness_install_matrix fail-fast stops after the first failing cell", () => {
  const tmpDir = fs.mkdtempSync(path.join(testTmpRoot(), "ctx-harness-install-matrix-"));
  const fixturePath = writeFixture(tmpDir);
  const smokePath = writeSmokeScript(tmpDir);
  const smokeArgs = path.join(tmpDir, "smoke-args.log");
  const result = runScript({
    fixturePath,
    smokePath,
    artifactsDir: path.join(tmpDir, "artifacts"),
    extraArgs: ["--lane", "release", "--platform", "linux", "--target", "host", "--provider", "gemini,codex", "--fail-fast"],
    extraEnv: { CTX_TEST_SMOKE_ARGS: smokeArgs },
  });

  assert.equal(result.status, 1);
  const argsLog = fs.readFileSync(smokeArgs, "utf8");
  assert.match(argsLog, /--provider codex/);
  assert.match(argsLog, /--provider gemini/);
});
