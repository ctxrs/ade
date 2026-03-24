const test = require("node:test");
const assert = require("node:assert/strict");

const {
  resolveBundledRuntimeIds,
  resolveMacOsBundlePodman,
} = require("./desktop_sync_resources.cjs");

test("resolveBundledRuntimeIds drops podman from default macOS bundle requirements", () => {
  assert.deepEqual(
    resolveBundledRuntimeIds(["python", "podman", "avf-linux-guest", "podman"], "darwin"),
    ["avf-linux-guest", "python"],
  );
});

test("resolveBundledRuntimeIds preserves podman on non-macOS platforms", () => {
  assert.deepEqual(
    resolveBundledRuntimeIds(["python", "podman", "python"], "linux"),
    ["podman", "python"],
  );
});

test("resolveMacOsBundlePodman defaults off and honors explicit opt-in", () => {
  assert.equal(resolveMacOsBundlePodman({}), false);
  assert.equal(resolveMacOsBundlePodman({ CTX_BUNDLE_PODMAN: "1" }), true);
  assert.equal(resolveMacOsBundlePodman({ CTX_BUNDLE_PODMAN: "off" }), false);
});

test("resolveMacOsBundlePodman rejects invalid override values", () => {
  assert.throws(
    () => resolveMacOsBundlePodman({ CTX_BUNDLE_PODMAN: "maybe" }),
    /CTX_BUNDLE_PODMAN/,
  );
});
