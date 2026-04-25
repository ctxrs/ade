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
  providers: [{ id: "codex-crp", owner: "provider-codex" }],
  auth_modes: [{ id: "endpoint_api_key", description: "api key auth" }],
  daemon_locations: [{ id: "local", description: "local daemon" }],
  execution_environments: [{ id: "sandbox", description: "sandbox" }],
  assertion_definitions: {
    install_success: "install ok",
    probe_success: "probe ok",
    model_list_population: "models ok",
    first_turn_success: "first turn ok",
    unsupported_contract: "unsupported contract",
  },
  cells: [
    {
      id: "codex.endpoint_api_key.local.sandbox",
      provider_id: "codex-crp",
      auth_mode: "endpoint_api_key",
      daemon_location: "local",
      execution_environment: "sandbox",
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

test("validate script accepts the legacy codex alias for required lane provider cells", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-auth-matrix-pass-codex-alias-"));
  const manifestPath = path.join(tmp, "provider_auth_matrix.json");
  const manifest = baseManifest();
  manifest.providers = [{ id: "codex", owner: "provider-codex" }];
  manifest.cells[0].provider_id = "codex";
  writeJson(manifestPath, manifest);

  const result = run(["--manifest", manifestPath]);
  assert.equal(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
});

test("validate script fails when required cross-product cell is missing", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-auth-matrix-fail-missing-"));
  const manifestPath = path.join(tmp, "provider_auth_matrix.json");
  const manifest = baseManifest();
  manifest.execution_environments.push({ id: "host", description: "host" });
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

test("validate script fails when a deferred concrete runner lacks an approved blocker reason", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-auth-matrix-fail-blocker-"));
  const manifestPath = path.join(tmp, "provider_auth_matrix.json");
  const manifest = baseManifest();
  manifest.cells[0].support = "deferred";
  manifest.cells[0].lane = "nightly";
  manifest.cells[0].skip_reason = "credential_backed_validation_pending";
  writeJson(manifestPath, manifest);

  const result = run(["--manifest", manifestPath]);
  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /requires skip_reason/i);
});

test("validate script fails when a required cell introduces a non-approved prerequisite", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-auth-matrix-fail-required-secret-"));
  const manifestPath = path.join(tmp, "provider_auth_matrix.json");
  const manifest = baseManifest();
  manifest.cells[0].prerequisites = ["OPENROUTER_API_KEY", "CTX_E2E_CURSOR_API_KEY"];
  writeJson(manifestPath, manifest);

  const result = run(["--manifest", manifestPath]);
  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /outside approved required-lane secret set/i);
});

test("validate script fails when a required Codex container cell loses the container scenario token", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-auth-matrix-fail-required-scenario-"));
  const manifestPath = path.join(tmp, "provider_auth_matrix.json");
  const manifest = baseManifest();
  manifest.cells[0].runner.scenarios = "local-codex-host-smoke";
  writeJson(manifestPath, manifest);

  const result = run(["--manifest", manifestPath]);
  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /local sandbox Codex required coverage must include local-codex-smoke/i);
});
