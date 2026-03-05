const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const childProcess = require("node:child_process");

const { validateReleaseGate } = require("./linux_arm_provider_release_gate.cjs");
const scriptPath = path.join(__dirname, "linux_arm_provider_release_gate.cjs");

const writeJson = (filePath, value) => {
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
};

const mkFixture = ({ manifest, report }) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-linux-arm-gate-"));
  const manifestPath = path.join(dir, "manifest.json");
  const reportPath = path.join(dir, "report.json");
  writeJson(manifestPath, manifest);
  writeJson(reportPath, report);
  return { manifestPath, reportPath };
};

const manifest = {
  version: 1,
  critical_providers: [{ provider_id: "codex" }, { provider_id: "goose" }],
  full_nightly_providers: [{ provider_id: "codex" }, { provider_id: "goose" }, { provider_id: "opencode" }],
};

test("release gate passes when all critical providers pass", () => {
  const { manifestPath, reportPath } = mkFixture({
    manifest,
    report: {
      results: [
        { provider_id: "codex", result: "pass" },
        { provider_id: "goose", result: "pass" },
      ],
    },
  });

  const result = validateReleaseGate({
    lane: "critical",
    matrixManifestPath: manifestPath,
    reportPath,
    strictMissing: true,
  });

  assert.equal(result.ok, true);
  assert.equal(result.errors.length, 0);
});

test("release gate fails when one critical provider fails", () => {
  const { manifestPath, reportPath } = mkFixture({
    manifest,
    report: {
      results: [
        { provider_id: "codex", result: "pass" },
        { provider_id: "goose", result: "fail", stage: "first_turn", error_code: "timeout", reason: "upstream timeout" },
      ],
    },
  });

  const result = validateReleaseGate({
    lane: "critical",
    matrixManifestPath: manifestPath,
    reportPath,
    strictMissing: true,
  });

  assert.equal(result.ok, false);
  assert.ok(result.errors.some((entry) => entry.includes("provider goose failed")));
});

test("release gate fails on missing provider when strictMissing=true", () => {
  const { manifestPath, reportPath } = mkFixture({
    manifest,
    report: {
      results: [{ provider_id: "codex", result: "pass" }],
    },
  });

  const result = validateReleaseGate({
    lane: "critical",
    matrixManifestPath: manifestPath,
    reportPath,
    strictMissing: true,
  });

  assert.equal(result.ok, false);
  assert.ok(result.errors.some((entry) => entry.includes("missing report row")));
});

test("release gate CLI supports explicit override with ticket", () => {
  const { manifestPath, reportPath } = mkFixture({
    manifest,
    report: {
      results: [
        { provider_id: "codex", result: "pass" },
        { provider_id: "goose", result: "fail", stage: "first_turn", error_code: "timeout", reason: "upstream timeout" },
      ],
    },
  });

  const run = childProcess.spawnSync(
    "node",
    [
      scriptPath,
      "--report",
      reportPath,
      "--matrix-manifest",
      manifestPath,
      "--lane",
      "critical",
      "--allow-override",
      "--override-ticket",
      "REL-1234",
    ],
    {
      encoding: "utf8",
      cwd: path.join(__dirname, ".."),
    },
  );

  assert.equal(run.status, 0, `stdout=${run.stdout}\nstderr=${run.stderr}`);
  assert.match(`${run.stdout}\n${run.stderr}`, /override applied/i);
});
