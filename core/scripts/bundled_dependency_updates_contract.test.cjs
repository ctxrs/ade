const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const childProcess = require("node:child_process");

const scriptPath = path.resolve(__dirname, "bundled_dependency_updates.cjs");
const scriptText = fs.readFileSync(scriptPath, "utf8");
const repoRoot = path.resolve(__dirname, "..", "..");
const { providerPolicyIssues } = require("./bundled_dependency_updates.cjs");
const providerMatrix = JSON.parse(
  fs.readFileSync(path.join(repoRoot, "core", "crates", "ctx-provider-accounts", "src", "provider_matrix.json"), "utf8"),
);

function findProvider(id) {
  const entry = providerMatrix.providers.find((provider) => provider && provider.id === id);
  assert.ok(entry, `expected provider ${id} in provider matrix fixture`);
  return JSON.parse(JSON.stringify(entry));
}

test("bundled dependency policy reads managed runtime versions from ctx-managed-installs", () => {
  assert.match(scriptText, /ctx-managed-installs", "src", "lib\.rs"/);
});

test("bundled dependency policy mode resolves current runtime constants without network access", () => {
  const result = childProcess.spawnSync(process.execPath, [scriptPath, "policy"], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr || result.stdout);
  assert.match(result.stdout, /^runtime:ok\tnode\t/m);
  assert.match(result.stdout, /^runtime:ok\tpython\t/m);
  assert.match(result.stdout, /^runtime:ok\tpython-build-tag\t/m);
});

test("bundled dependency policy ignores codex archive_unresolved when provenance is valid", () => {
  const issues = providerPolicyIssues({
    providerId: "codex",
    entry: findProvider("codex"),
    latestInfo: { resolver: "archive_unresolved" },
    upstreamInfo: null,
    currentUpstreamVersion: "rust-v0.114.0",
    policyModeEnabled: false,
  });

  assert.deepEqual(issues, []);
});

test("bundled dependency policy still flags broken codex provenance", () => {
  const entry = findProvider("codex");
  entry.releases[0].provenance.upstream_repo = "";

  const issues = providerPolicyIssues({
    providerId: "codex",
    entry,
    latestInfo: { resolver: "archive_unresolved" },
    upstreamInfo: null,
    currentUpstreamVersion: "rust-v0.114.0",
    policyModeEnabled: false,
  });

  assert.match(issues.join("\n"), /codex provenance\.upstream_repo is missing or invalid/);
  assert.match(issues.join("\n"), /archive source resolver is unresolved/);
});

test("bundled dependency policy still flags unresolved non-codex archives", () => {
  const issues = providerPolicyIssues({
    providerId: "goose",
    entry: findProvider("goose"),
    latestInfo: { resolver: "archive_unresolved" },
    upstreamInfo: null,
    currentUpstreamVersion: "1.31.1",
    policyModeEnabled: false,
  });

  assert.match(issues.join("\n"), /archive source resolver is unresolved/);
});
