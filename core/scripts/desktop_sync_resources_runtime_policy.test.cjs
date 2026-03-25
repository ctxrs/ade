const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  resolveBundledRuntimeIds,
  stageAvfLinuxGuestRuntime,
} = require("./desktop_sync_resources.cjs");

test("resolveBundledRuntimeIds dedupes and sorts bundle runtime identifiers", () => {
  assert.deepEqual(
    resolveBundledRuntimeIds(["python", "avf-linux-guest", "python", "zeta"]),
    ["avf-linux-guest", "python", "zeta"],
  );
});

test("stageAvfLinuxGuestRuntime leaves the bundle untouched when no guest artifact is present", () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-bundle-"));
  const staged = stageAvfLinuxGuestRuntime(bundleDir);
  assert.equal(staged, false);
  assert.equal(fs.existsSync(path.join(bundleDir, "runtimes", "avf-linux-guest")), false);
});
