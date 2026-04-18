const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const packageJson = JSON.parse(fs.readFileSync(path.join(coreRoot, "package.json"), "utf8"));
const anomalyScript = fs.readFileSync(path.join(coreRoot, "scripts", "run-anomaly-suite.sh"), "utf8");
const fuzzScript = fs.readFileSync(path.join(coreRoot, "scripts", "run-fuzz-regression.sh"), "utf8");

function escapeRegex(value) {
  return String(value).replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}

test("resilience suites expose split anomaly and fuzz regression lanes", () => {
  assert.match(anomalyScript, /usage: \$0 \{all\|ctx-http-fault-matrix\|ctx-http-hot-endpoints-no-db\|ctx-store-fault-injection\}/);
  assert.match(anomalyScript, /ctx-http-fault-matrix\)/);
  assert.match(anomalyScript, /ctx-http-hot-endpoints-no-db\)/);
  assert.match(anomalyScript, /ctx-store-fault-injection\)/);

  assert.equal(
    packageJson.scripts["verify:anomaly"],
    "pnpm verify:anomaly:ctx-http:fault-matrix && pnpm verify:anomaly:ctx-http:hot-endpoints-no-db && pnpm verify:anomaly:ctx-store:fault-injection",
  );
  assert.equal(
    packageJson.scripts["verify:anomaly:ctx-http:fault-matrix"],
    "bash -lc 'scripts/run-anomaly-suite.sh ctx-http-fault-matrix'",
  );
  assert.equal(
    packageJson.scripts["verify:anomaly:ctx-http:hot-endpoints-no-db"],
    "bash -lc 'scripts/run-anomaly-suite.sh ctx-http-hot-endpoints-no-db'",
  );
  assert.equal(
    packageJson.scripts["verify:anomaly:ctx-store:fault-injection"],
    "bash -lc 'scripts/run-anomaly-suite.sh ctx-store-fault-injection'",
  );

  assert.match(fuzzScript, /usage: \$0 \{all\|providers\|mcp\|workspace-payloads\|release-manifests\|desktop-ipc\}/);
  for (const lane of [
    "providers",
    "mcp",
    "workspace-payloads",
    "release-manifests",
    "desktop-ipc",
  ]) {
    assert.match(fuzzScript, new RegExp(`${escapeRegex(lane)}\\)`));
  }

  assert.equal(
    packageJson.scripts["verify:fuzz:regression"],
    "pnpm verify:fuzz:regression:providers && pnpm verify:fuzz:regression:mcp && pnpm verify:fuzz:regression:workspace-payloads && pnpm verify:fuzz:regression:release-manifests && pnpm verify:fuzz:regression:desktop-ipc",
  );
  assert.equal(
    packageJson.scripts["verify:fuzz:regression:providers"],
    "bash -lc 'scripts/run-fuzz-regression.sh providers'",
  );
  assert.equal(
    packageJson.scripts["verify:fuzz:regression:mcp"],
    "bash -lc 'scripts/run-fuzz-regression.sh mcp'",
  );
  assert.equal(
    packageJson.scripts["verify:fuzz:regression:workspace-payloads"],
    "bash -lc 'scripts/run-fuzz-regression.sh workspace-payloads'",
  );
  assert.equal(
    packageJson.scripts["verify:fuzz:regression:release-manifests"],
    "bash -lc 'scripts/run-fuzz-regression.sh release-manifests'",
  );
  assert.equal(
    packageJson.scripts["verify:fuzz:regression:desktop-ipc"],
    "bash -lc 'scripts/run-fuzz-regression.sh desktop-ipc'",
  );
});
