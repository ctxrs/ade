const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const childProcess = require("node:child_process");

const scriptPath = path.join(__dirname, "provider_matrix_archive_gap_report.cjs");

const writeJson = (filePath, value) => {
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
};

const run = (args, opts = {}) =>
  childProcess.spawnSync("node", [scriptPath, ...args], {
    encoding: "utf8",
    cwd: opts.cwd || path.join(__dirname, ".."),
  });

test("archive gap report passes when required targets+sha are present", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-gap-pass-"));
  const matrixPath = path.join(tmp, "provider_matrix.json");

  writeJson(matrixPath, {
    version: 2,
    providers: [
      {
        id: "amp",
        managed_install: {
          kind: "archive",
          version: "0.1.0",
          targets: {
            "darwin-aarch64": { url: "https://example/a", archive: "tar_gz", bin_path: "amp-acp", sha256: "a".repeat(64) },
            "darwin-x86_64": { url: "https://example/b", archive: "tar_gz", bin_path: "amp-acp", sha256: "b".repeat(64) },
            "linux-aarch64": { url: "https://example/c", archive: "tar_gz", bin_path: "amp-acp", sha256: "c".repeat(64) },
            "linux-x86_64": { url: "https://example/d", archive: "tar_gz", bin_path: "amp-acp", sha256: "d".repeat(64) },
          },
        },
      },
    ],
  });

  const result = run(["--matrix", matrixPath, "--include-providers", "amp"]);
  assert.equal(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
});

test("archive gap report fails when required target is missing", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-gap-fail-"));
  const matrixPath = path.join(tmp, "provider_matrix.json");

  writeJson(matrixPath, {
    version: 2,
    providers: [
      {
        id: "amp",
        managed_install: {
          kind: "archive",
          version: "0.1.0",
          targets: {
            "darwin-aarch64": { url: "https://example/a", archive: "tar_gz", bin_path: "amp-acp", sha256: "a".repeat(64) },
          },
        },
      },
    ],
  });

  const result = run(["--matrix", matrixPath, "--include-providers", "amp"]);
  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /missing targets/i);
});

test("archive gap report resolves --matrix relative to caller cwd", () => {
  const repoRoot = path.resolve(__dirname, "..", "..");
  const relativeMatrixPath = path.join("core", "crates", "ctx-provider-accounts", "src", "provider_matrix.json");
  const result = run(["--matrix", relativeMatrixPath, "--include-providers", "acp-crp-bridge"], { cwd: repoRoot });
  assert.notEqual(result.status, null, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.doesNotMatch(result.stderr, /ENOENT|no such file/i, `stdout=${result.stdout}\nstderr=${result.stderr}`);
});
