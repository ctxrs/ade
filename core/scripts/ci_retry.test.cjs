const assert = require("node:assert/strict");
const childProcess = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const REPO_ROOT = path.resolve(__dirname, "..", "..");
const CI_RETRY_SCRIPT = path.join(REPO_ROOT, "scripts", "ci_retry.sh");

function writeHelperCommand(tmpDir) {
  const helperPath = path.join(tmpDir, "helper.sh");
  fs.writeFileSync(
    helperPath,
    [
      "#!/usr/bin/env bash",
      "set -euo pipefail",
      "mode=\"$1\"",
      "count_file=\"$2\"",
      "count=0",
      "if [[ -f \"$count_file\" ]]; then",
      "  count=\"$(cat \"$count_file\")\"",
      "fi",
      "count=$((count + 1))",
      "printf '%s' \"$count\" > \"$count_file\"",
      "case \"$mode\" in",
      "  always-fail)",
      "    exit 7",
      "    ;;",
      "  fail-once)",
      "    if [[ \"$count\" -eq 1 ]]; then",
      "      exit 9",
      "    fi",
      "    ;;",
      "  *)",
      "    echo \"unexpected mode: $mode\" >&2",
      "    exit 2",
      "    ;;",
      "esac",
    ].join("\n"),
    { mode: 0o755 },
  );
  return helperPath;
}

function runCiRetry(mode) {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-ci-retry-"));
  const logFile = path.join(tmpDir, "flake-stats.jsonl");
  const countFile = path.join(tmpDir, "attempt-count.txt");
  const helperPath = writeHelperCommand(tmpDir);

  const result = childProcess.spawnSync(
    "bash",
    [CI_RETRY_SCRIPT, helperPath, mode, countFile],
    {
      cwd: REPO_ROOT,
      encoding: "utf8",
      env: {
        ...process.env,
        CI_FLAKE_LOG: logFile,
        CI_JOB_NAME: "ci-retry-test",
        CI_RETRY_ATTEMPTS: "2",
        CI_RETRY_DELAY_SEC: "0",
      },
    },
  );

  const logLines = fs
    .readFileSync(logFile, "utf8")
    .trim()
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line));

  return {
    attemptCount: fs.readFileSync(countFile, "utf8").trim(),
    logLines,
    result,
  };
}

test("ci_retry exits with the failing command status after exhausting retries", () => {
  const { attemptCount, logLines, result } = runCiRetry("always-fail");

  assert.equal(result.status, 7);
  assert.equal(attemptCount, "2");
  assert.equal(logLines.length, 1);
  assert.equal(logLines[0].status, "failed");
  assert.equal(logLines[0].attempts, 2);
  assert.equal(logLines[0].exit_code, 7);
  assert.equal(logLines[0].job, "ci-retry-test");
  assert.match(logLines[0].command, /helper\.sh always-fail /);
});

test("ci_retry records a flake and exits zero when a retry succeeds", () => {
  const { attemptCount, logLines, result } = runCiRetry("fail-once");

  assert.equal(result.status, 0);
  assert.equal(attemptCount, "2");
  assert.equal(logLines.length, 1);
  assert.equal(logLines[0].status, "flake");
  assert.equal(logLines[0].attempts, 2);
  assert.equal(logLines[0].exit_code, 9);
  assert.equal(logLines[0].job, "ci-retry-test");
});
