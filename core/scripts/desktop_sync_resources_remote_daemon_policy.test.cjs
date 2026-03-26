const test = require("node:test");
const assert = require("node:assert/strict");

const {
  shouldBundleRemoteDaemons,
} = require("./desktop_sync_resources_remote_daemon_policy.cjs");
const {
  __desktopSyncResourcesTestHooks,
} = require("./desktop_sync_resources.cjs");

test("remote daemon bundling defaults to enabled", () => {
  assert.equal(shouldBundleRemoteDaemons({}), true);
});

test("remote daemon bundling can be disabled with explicit toggle values", () => {
  for (const value of ["0", "false", "no", "off", " OFF "]) {
    assert.equal(shouldBundleRemoteDaemons({ CTX_BUNDLE_REMOTE_DAEMONS: value }), false);
  }
});

test("remote daemon bundling remains enabled for explicit true values", () => {
  for (const value of ["1", "true", "yes", "on"]) {
    assert.equal(shouldBundleRemoteDaemons({ CTX_BUNDLE_REMOTE_DAEMONS: value }), true);
  }
});

test("remote daemon bundling rejects invalid values", () => {
  assert.throws(
    () => shouldBundleRemoteDaemons({ CTX_BUNDLE_REMOTE_DAEMONS: "random" }),
    /Invalid CTX_BUNDLE_REMOTE_DAEMONS/,
  );
});

test("remote daemon container build command creates /out before install", () => {
  const [spawnCmd, args] = __desktopSyncResourcesTestHooks.buildRemoteDaemonContainerArgs({
    runtime: "docker",
    builderImage: "rust:test",
    coreDir: "/src-host",
    daemonsDir: "/out-host",
    targetCache: "/target-host",
    cargoRegistryCache: "/registry-host",
    cargoGitCache: "/git-host",
    target: {
      platform: "linux/amd64",
      rustTarget: "x86_64-unknown-linux-gnu",
      fileName: "ctx-daemon-linux-x86_64",
    },
  });

  assert.equal(spawnCmd, "docker");
  assert.match(args.join(" "), /mkdir -p \/out;/);
  assert.match(args.join(" "), /install -Dm0755 .* \/out\/ctx-daemon-linux-x86_64/);
});
