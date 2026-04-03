const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  __desktopSyncResourcesTestHooks: { filterManagedAvfLocalPayloadErrors },
} = require("./desktop_sync_resources.cjs");

const writeLock = (dir, component) => {
  fs.writeFileSync(
    path.join(dir, "runtime_lock.v2.json"),
    `${JSON.stringify({ version: 2, components: [component] }, null, 2)}\n`,
    "utf8",
  );
};

const managedAvfComponent = () => ({
  kind: "runtime",
  id: "avf-linux-guest",
  os: "macos",
  arch: "aarch64",
  variant: "default",
  sources: [
    {
      source_type: "ci",
      uri: "https://example.invalid/rootfs.raw.zst",
      sha256: "a".repeat(64),
    },
  ],
  helpers: {
    kernel: { uri: "https://example.invalid/kernel", sha256: "b".repeat(64) },
    initrd: { uri: "https://example.invalid/initrd", sha256: "c".repeat(64) },
    "guest-agent": { uri: "https://example.invalid/guest-agent", sha256: "d".repeat(64) },
    "egress-proxy": { uri: "https://example.invalid/egress-proxy", sha256: "e".repeat(64) },
    "container-stack": { uri: "https://example.invalid/container-stack", sha256: "f".repeat(64) },
  },
});

test("managed AVF local payload errors can be filtered when the runtime lock has complete managed metadata", () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-managed-avf-policy-"));
  writeLock(bundleDir, managedAvfComponent());
  const errors = [
    "runtime root avf-linux-guest (macos/aarch64) missing directory: /tmp/runtime",
    "runtime bin avf-linux-guest (macos/aarch64) missing file: /tmp/runtime/rootfs.raw",
    "runtime helper avf-linux-guest/kernel (macos/aarch64) missing file: /tmp/runtime/helpers/kernel",
    "missing provider bundle entry for codex (linux/x86_64)",
  ];

  const filtered = filterManagedAvfLocalPayloadErrors({
    errors,
    bundleDir,
    hostOs: "macos",
    hostArch: "aarch64",
    allowManagedRuntime: true,
  });

  assert.deepEqual(filtered, ["missing provider bundle entry for codex (linux/x86_64)"]);
});

test("managed AVF local payload errors are preserved when the runtime lock lacks complete managed metadata", () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-managed-avf-policy-"));
  const component = managedAvfComponent();
  delete component.helpers.kernel;
  writeLock(bundleDir, component);
  const errors = [
    "runtime root avf-linux-guest (macos/aarch64) missing directory: /tmp/runtime",
    "runtime helper avf-linux-guest/kernel (macos/aarch64) missing file: /tmp/runtime/helpers/kernel",
  ];

  const filtered = filterManagedAvfLocalPayloadErrors({
    errors,
    bundleDir,
    hostOs: "macos",
    hostArch: "aarch64",
    allowManagedRuntime: true,
  });

  assert.deepEqual(filtered, errors);
});

test("managed AVF local payload errors can be filtered for macos x64 hosts when the failing target is macos aarch64", () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-managed-avf-policy-"));
  writeLock(bundleDir, managedAvfComponent());
  const errors = [
    "runtime root avf-linux-guest (macos/aarch64) missing directory: /tmp/runtime",
    "runtime helper avf-linux-guest/kernel (macos/aarch64) missing file: /tmp/runtime/helpers/kernel",
    "missing provider bundle entry for codex (linux/x86_64)",
  ];

  const filtered = filterManagedAvfLocalPayloadErrors({
    errors,
    bundleDir,
    hostOs: "macos",
    hostArch: "x86_64",
    allowManagedRuntime: true,
  });

  assert.deepEqual(filtered, ["missing provider bundle entry for codex (linux/x86_64)"]);
});

test("managed AVF local payload errors are preserved when the runtime lock does not cover the failing target arch", () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-managed-avf-policy-"));
  writeLock(bundleDir, managedAvfComponent());
  const errors = [
    "runtime root avf-linux-guest (macos/x86_64) missing directory: /tmp/runtime",
    "runtime helper avf-linux-guest/kernel (macos/x86_64) missing file: /tmp/runtime/helpers/kernel",
  ];

  const filtered = filterManagedAvfLocalPayloadErrors({
    errors,
    bundleDir,
    hostOs: "macos",
    hostArch: "x86_64",
    allowManagedRuntime: true,
  });

  assert.deepEqual(filtered, errors);
});
