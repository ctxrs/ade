const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const childProcess = require("node:child_process");

const scriptPath = path.join(__dirname, "provider_matrix_required_targets_gate.cjs");

const writeJson = (filePath, value) => {
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
};

const run = (args, opts = {}) =>
  childProcess.spawnSync("node", [scriptPath, ...args], {
    encoding: "utf8",
    cwd: opts.cwd || path.join(__dirname, ".."),
  });

const fullArchiveTargets = (binPath = "bin/provider") => ({
  "darwin-aarch64": { url: "https://example/darwin-aarch64", archive: "tar_gz", bin_path: binPath, sha256: "a".repeat(64) },
  "darwin-x86_64": { url: "https://example/darwin-x86_64", archive: "tar_gz", bin_path: binPath, sha256: "b".repeat(64) },
  "linux-aarch64": { url: "https://example/linux-aarch64", archive: "tar_gz", bin_path: binPath, sha256: "c".repeat(64) },
  "linux-x86_64": { url: "https://example/linux-x86_64", archive: "tar_gz", bin_path: binPath, sha256: "d".repeat(64) },
});

const fullHybridTargets = (binPath = "provider") => ({
  "darwin-aarch64": { url: "https://example/hybrid-darwin-aarch64", archive: "tar_gz", bin_path: binPath, sha256: "1".repeat(64) },
  "darwin-x86_64": { url: "https://example/hybrid-darwin-x86_64", archive: "tar_gz", bin_path: binPath, sha256: "2".repeat(64) },
  "linux-aarch64": { url: "https://example/hybrid-linux-aarch64", archive: "tar_gz", bin_path: binPath, sha256: "3".repeat(64) },
  "linux-x86_64": { url: "https://example/hybrid-linux-x86_64", archive: "tar_gz", bin_path: binPath, sha256: "4".repeat(64) },
});

const writeFixture = (dir, matrixProviders, lockProviderIds = []) => {
  const matrixPath = path.join(dir, "provider_matrix.json");
  const lockPath = path.join(dir, "runtime_lock.v2.json");
  writeJson(matrixPath, { version: 2, providers: matrixProviders });
  writeJson(lockPath, { version: 2, required: { provider_ids: lockProviderIds } });
  return { matrixPath, lockPath };
};

test("required targets gate uses release planner fallback when lock provider_ids is empty", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-target-gate-fallback-"));
  const { matrixPath, lockPath } = writeFixture(tmp, [
    {
      id: "amp",
      managed_install: {
        kind: "archive",
        version: "1.0.0",
        targets: fullArchiveTargets("dist/bin/amp-acp.js"),
      },
    },
    {
      id: "cursor",
      managed_install: {
        kind: "npm",
        package: "@example/cursor",
        version: "1.0.0",
        entrypoint: "cursor",
        targets: fullHybridTargets("cursor"),
      },
    },
  ]);

  const result = run([
    "--lock",
    lockPath,
    "--matrix",
    matrixPath,
    "--include-kinds",
    "archive,npm",
  ]);

  assert.equal(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stdout, /2 providers checked/i);
});

test("required targets gate fails on missing target even with empty lock provider_ids", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-target-gate-missing-"));
  const archiveTargets = fullArchiveTargets("dist/bin/codex-crp");
  delete archiveTargets["linux-x86_64"];
  const { matrixPath, lockPath } = writeFixture(tmp, [
    {
      id: "codex-crp",
      managed_install: {
        kind: "archive",
        version: "1.0.0",
        targets: archiveTargets,
      },
    },
  ]);

  const result = run([
    "--lock",
    lockPath,
    "--matrix",
    matrixPath,
    "--include-kinds",
    "archive",
  ]);

  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /provider codex .*missing target/i, `stdout=${result.stdout}\nstderr=${result.stderr}`);
});

test("required targets gate falls back to managed providers when release planner defaults are absent", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-target-gate-managed-fallback-"));
  const { matrixPath, lockPath } = writeFixture(tmp, [
    {
      id: "custom-provider",
      managed_install: {
        kind: "archive",
        version: "1.0.0",
        targets: fullArchiveTargets("bin/custom-provider"),
      },
    },
  ]);

  const result = run([
    "--lock",
    lockPath,
    "--matrix",
    matrixPath,
    "--include-kinds",
    "archive",
  ]);

  assert.equal(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stdout, /1 providers checked/i);
});
