const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  loadMatrix,
  providerIdsForLane,
  validateMatrix,
} = require("./linux_arm_provider_reliability_matrix.cjs");

const writeMatrix = (value) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-linux-arm-matrix-"));
  const filePath = path.join(dir, "matrix.json");
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
  return filePath;
};

test("loadMatrix validates default fixture and exposes critical/nightly ids", () => {
  const { matrix } = loadMatrix();
  const critical = providerIdsForLane(matrix, "critical");
  const nightly = providerIdsForLane(matrix, "nightly");

  assert.ok(critical.length > 0);
  assert.ok(nightly.length >= critical.length);
  for (const providerId of critical) {
    assert.ok(nightly.includes(providerId), `nightly missing critical provider ${providerId}`);
  }
});

test("validateMatrix rejects duplicate provider ids", () => {
  const fixture = {
    version: 1,
    critical_providers: [{ provider_id: "codex" }, { provider_id: "codex" }],
    full_nightly_providers: [{ provider_id: "codex" }],
  };
  assert.throws(() => validateMatrix(fixture), /duplicate provider_id/i);
});

test("validateMatrix rejects critical provider outside nightly set", () => {
  const fixture = {
    version: 1,
    critical_providers: [{ provider_id: "codex" }],
    full_nightly_providers: [{ provider_id: "opencode" }],
  };
  assert.throws(() => validateMatrix(fixture), /missing from full_nightly_providers/i);
});

test("loadMatrix reads custom path", () => {
  const matrixPath = writeMatrix({
    version: 1,
    critical_providers: [{ provider_id: "codex" }],
    full_nightly_providers: [{ provider_id: "codex" }, { provider_id: "opencode" }],
    deferred_providers: [{ provider_id: "qwen", reason: "missing linux-aarch64" }],
    unsupported_providers: ["junie"],
  });
  const { matrix } = loadMatrix(matrixPath);
  assert.deepEqual(providerIdsForLane(matrix, "critical"), ["codex"]);
  assert.deepEqual(providerIdsForLane(matrix, "nightly"), ["codex", "opencode"]);
});
