const test = require("node:test");
const assert = require("node:assert/strict");

const {
  shouldBundleRemoteDaemons,
} = require("./desktop_sync_resources_remote_daemon_policy.cjs");

test("remote daemon bundling defaults to enabled", () => {
  assert.equal(shouldBundleRemoteDaemons({}), true);
});

test("remote daemon bundling can be disabled with explicit toggle values", () => {
  for (const value of ["0", "false", "no", "off", " OFF "]) {
    assert.equal(shouldBundleRemoteDaemons({ CTX_BUNDLE_REMOTE_DAEMONS: value }), false);
  }
});

test("remote daemon bundling remains enabled for non-disable values", () => {
  for (const value of ["1", "true", "yes", "on", "random"]) {
    assert.equal(shouldBundleRemoteDaemons({ CTX_BUNDLE_REMOTE_DAEMONS: value }), true);
  }
});
