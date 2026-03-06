const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { runPreflight } = require("./linux_arm_provider_preflight.cjs");

const writeJson = (filePath, value) => {
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
};

const mkFixtureDir = () => fs.mkdtempSync(path.join(os.tmpdir(), "ctx-linux-arm-preflight-"));

const baseManifest = {
  version: 1,
  critical_providers: [{ provider_id: "codex" }],
  full_nightly_providers: [{ provider_id: "codex" }],
};

const baseProviderMatrix = {
  version: 2,
  providers: [
    {
      id: "codex",
      managed_install: {
        kind: "archive",
        targets: {
          "linux-aarch64": {
            url: "https://example.invalid/providers/codex/linux-aarch64/codex-linux-aarch64.tar.gz",
            archive: "tar_gz",
            bin_path: "bin/codex",
            sha256: "a".repeat(64),
            size_bytes: 12345,
          },
        },
      },
    },
  ],
};

const baseRuntimeLock = {
  version: 2,
  components: [
    {
      kind: "provider",
      id: "codex",
      os: "linux",
      arch: "aarch64",
      sources: [{ source_type: "ci", uri: "locked://providers/codex/linux/aarch64", sha256: "0".repeat(64) }],
    },
    {
      kind: "runtime",
      id: "node",
      os: "linux",
      arch: "aarch64",
      sources: [{ source_type: "vendor", uri: "locked://runtimes/node/linux/aarch64", sha256: "0".repeat(64) }],
    },
    {
      kind: "runtime",
      id: "python",
      os: "linux",
      arch: "aarch64",
      sources: [{ source_type: "vendor", uri: "locked://runtimes/python/linux/aarch64", sha256: "0".repeat(64) }],
    },
    {
      kind: "image",
      id: "ctx-harness",
      os: "linux",
      arch: "aarch64",
      sources: [{ source_type: "ci", uri: "https://example.invalid/images/ctx-harness/linux/aarch64/ctx-harness.tar", sha256: "b".repeat(64) }],
    },
  ],
};

const run = ({ manifest = baseManifest, providerMatrix = baseProviderMatrix, runtimeLock = baseRuntimeLock, lane = "critical", strictSizeBytes = false }) => {
  const dir = mkFixtureDir();
  const manifestPath = path.join(dir, "manifest.json");
  const providerMatrixPath = path.join(dir, "provider_matrix.json");
  const runtimeLockPath = path.join(dir, "runtime_lock.v2.json");
  writeJson(manifestPath, manifest);
  writeJson(providerMatrixPath, providerMatrix);
  writeJson(runtimeLockPath, runtimeLock);

  return runPreflight({
    lane,
    matrixManifestPath: manifestPath,
    providerMatrixPath,
    runtimeLockPath,
    strictSizeBytes,
  });
};

test("linux arm preflight passes for valid fixtures", () => {
  const report = run({});
  assert.equal(report.summary.errors_total, 0);
  assert.equal(report.summary.providers_failed, 0);
  assert.equal(report.summary.runtime_failed, 0);
});

test("linux arm preflight fails when provider target is missing", () => {
  const providerMatrix = structuredClone(baseProviderMatrix);
  delete providerMatrix.providers[0].managed_install.targets["linux-aarch64"];
  const report = run({ providerMatrix });
  assert.ok(report.summary.errors_total > 0);
  assert.ok(report.errors.some((entry) => entry.includes("missing managed_install target linux-aarch64")));
});

test("linux arm preflight fails when provider url points to x86_64 artifact", () => {
  const providerMatrix = structuredClone(baseProviderMatrix);
  providerMatrix.providers[0].managed_install.targets["linux-aarch64"].url = "https://example.invalid/providers/codex/linux-x86_64/codex-linux-x86_64.tar.gz";
  const report = run({ providerMatrix });
  assert.ok(report.summary.errors_total > 0);
  assert.ok(report.errors.some((entry) => entry.includes("suggests non-arm64 artifact")));
});

test("linux arm preflight fails when runtime lock missing ctx-harness image", () => {
  const runtimeLock = structuredClone(baseRuntimeLock);
  runtimeLock.components = runtimeLock.components.filter((entry) => !(entry.kind === "image" && entry.id === "ctx-harness"));
  const report = run({ runtimeLock });
  assert.ok(report.summary.errors_total > 0);
  assert.ok(report.errors.some((entry) => entry.includes("missing runtime lock component image/ctx-harness")));
});

test("linux arm preflight can enforce size_bytes strictly", () => {
  const providerMatrix = structuredClone(baseProviderMatrix);
  delete providerMatrix.providers[0].managed_install.targets["linux-aarch64"].size_bytes;
  const report = run({ providerMatrix, strictSizeBytes: true });
  assert.ok(report.summary.errors_total > 0);
  assert.ok(report.errors.some((entry) => entry.includes("size_bytes")));
});

test("linux arm preflight validates linux-arm archive dependencies", () => {
  const providerMatrix = structuredClone(baseProviderMatrix);
  providerMatrix.providers[0].dependencies = [
    {
      id: "droid-cli",
      install: {
        kind: "archive",
        version: "0.69.0",
        targets: {},
      },
    },
  ];
  const report = run({ providerMatrix });
  assert.ok(report.summary.errors_total > 0);
  assert.ok(report.errors.some((entry) => entry.includes("dependency droid-cli")));
  assert.ok(report.errors.some((entry) => entry.includes("linux-aarch64")));
});
