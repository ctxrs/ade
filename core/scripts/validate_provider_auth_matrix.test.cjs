const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const childProcess = require("node:child_process");

const scriptPath = path.join(__dirname, "validate_provider_auth_matrix.cjs");

const run = (args, opts = {}) =>
  childProcess.spawnSync("node", [scriptPath, ...args], {
    encoding: "utf8",
    cwd: opts.cwd || path.join(__dirname, ".."),
  });

const writeJson = (filePath, value) => {
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
};

const baseManifest = () => ({
  schema_version: 1,
  generated_at: "2026-03-05",
  summary: "test manifest",
  providers: [{ id: "codex", owner: "provider-codex" }],
  auth_modes: [{ id: "endpoint_api_key", description: "api key auth" }],
  env_targets: [{ id: "local_container", description: "local container" }],
  assertion_definitions: {
    install_success: "install ok",
    probe_success: "probe ok",
    model_list_population: "models ok",
    first_turn_success: "first turn ok",
    unsupported_contract: "unsupported contract",
  },
  cells: [
    {
      id: "codex.endpoint_api_key.local_container",
      provider_id: "codex",
      auth_mode: "endpoint_api_key",
      env_target: "local_container",
      support: "supported",
      lane: "required",
      required_assertions: [
        "install_success",
        "probe_success",
        "model_list_population",
        "first_turn_success",
      ],
      owner: "provider-codex",
      prerequisites: ["OPENROUTER_API_KEY"],
      runner: {
        kind: "desktop_wdio",
        spec: "automation/specs/provider-auth-matrix-cell.spec.cjs",
        scenarios: "local-codex-smoke",
      },
      skip_reason: null,
    },
  ],
});

test("validate script passes for a minimal valid manifest", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-auth-matrix-pass-"));
  const manifestPath = path.join(tmp, "provider_auth_matrix.json");
  const reportPath = path.join(tmp, "provider_auth_matrix.md");
  writeJson(manifestPath, baseManifest());

  const first = run(["--manifest", manifestPath, "--report", reportPath]);
  assert.equal(first.status, 0, `stdout=${first.stdout}\nstderr=${first.stderr}`);
  assert.match(first.stdout, /validation passed/i);
  assert.ok(fs.existsSync(reportPath), "expected report to be written");

  const second = run(["--manifest", manifestPath, "--check-report", reportPath]);
  assert.equal(second.status, 0, `stdout=${second.stdout}\nstderr=${second.stderr}`);
});

test("validate script fails when required cross-product cell is missing", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-auth-matrix-fail-missing-"));
  const manifestPath = path.join(tmp, "provider_auth_matrix.json");
  const manifest = baseManifest();
  manifest.env_targets.push({ id: "local_host", description: "local host" });
  writeJson(manifestPath, manifest);

  const result = run(["--manifest", manifestPath]);
  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /missing matrix cell/i);
});

test("validate script fails when report content is out of date", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-auth-matrix-fail-report-"));
  const manifestPath = path.join(tmp, "provider_auth_matrix.json");
  const reportPath = path.join(tmp, "provider_auth_matrix.md");
  writeJson(manifestPath, baseManifest());
  fs.writeFileSync(reportPath, "# stale\n", "utf8");

  const result = run(["--manifest", manifestPath, "--check-report", reportPath]);
  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /report out of date/i);
});
