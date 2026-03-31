const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  expectedBundleRoot,
  parseVersionFile,
  validateAvfRuntimeFreshness,
} = require("./avf_runtime_lock_freshness.cjs");

test("parseVersionFile reads key-value metadata from version.txt", () => {
  const parsed = parseVersionFile(
    [
      "version=ubuntu-noble-arm64-deadbeef0000",
      "rootfs-sha256=1111",
      "",
      "# comment",
      "guest-agent-sha256=2222",
    ].join("\n"),
  );
  assert.equal(parsed.version, "ubuntu-noble-arm64-deadbeef0000");
  assert.equal(parsed["rootfs-sha256"], "1111");
  assert.equal(parsed["guest-agent-sha256"], "2222");
});

test("validateAvfRuntimeFreshness accepts a matching prepared runtime and bundled metadata", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-avf-runtime-freshness-"));
  const runtimeDir = path.join(tmpRoot, "runtime");
  const bundleDir = path.join(tmpRoot, "bundles");
  const version = "ubuntu-noble-arm64-d4620081a34d";
  const root = expectedBundleRoot(version);
  fs.mkdirSync(runtimeDir, { recursive: true });
  fs.mkdirSync(path.join(bundleDir, root), { recursive: true });
  fs.writeFileSync(
    path.join(runtimeDir, "version.txt"),
    [
      `version=${version}`,
      "rootfs-sha256=rootfs-sha",
      "kernel-sha256=kernel-sha",
      "initrd-sha256=initrd-sha",
      "guest-agent-sha256=guest-agent-sha",
      "egress-proxy-sha256=egress-sha",
      "container-stack-sha256=container-stack-sha",
      "",
    ].join("\n"),
    "utf8",
  );

  const manifest = {
    version: 1,
    providers: [],
    runtimes: [
      {
        id: "avf-linux-guest",
        version,
        os: "macos",
        arch: "aarch64",
        sha256: "rootfs-sha",
        root,
        bin: "rootfs.raw",
      },
    ],
    images: [],
  };
  const lock = {
    version: 2,
    components: [
      {
        kind: "runtime",
        id: "avf-linux-guest",
        os: "macos",
        arch: "aarch64",
        variant: "default",
        version,
        helpers: {
          kernel: { uri: `https://example.invalid/${version}/macos/aarch64/sha256-kernel/kernel`, sha256: "kernel-sha" },
          initrd: { uri: `https://example.invalid/${version}/macos/aarch64/sha256-initrd/initrd`, sha256: "initrd-sha" },
          "guest-agent": { uri: `https://example.invalid/${version}/macos/aarch64/sha256-guest/guest-agent`, sha256: "guest-agent-sha" },
          "egress-proxy": { uri: `https://example.invalid/${version}/macos/aarch64/sha256-egress/egress-proxy`, sha256: "egress-sha" },
          "container-stack": { uri: `https://example.invalid/${version}/macos/aarch64/sha256-stack/container-stack.tar.gz`, sha256: "container-stack-sha" },
        },
        sources: [
          { source_type: "ci", uri: `https://example.invalid/${version}/macos/aarch64/sha256-rootfs/rootfs.raw.zst`, sha256: "rootfs-zst-sha" },
        ],
      },
    ],
  };

  const result = validateAvfRuntimeFreshness({ runtimeDir, bundleDir, manifest, lock });
  assert.equal(result.ok, true, result.errors.join("\n"));

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});

test("validateAvfRuntimeFreshness reports guest runtime drift clearly", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-avf-runtime-freshness-drift-"));
  const runtimeDir = path.join(tmpRoot, "runtime");
  const bundleDir = path.join(tmpRoot, "bundles");
  const version = "ubuntu-noble-arm64-d4620081a34d";
  const root = expectedBundleRoot(version);
  fs.mkdirSync(runtimeDir, { recursive: true });
  fs.mkdirSync(path.join(bundleDir, root), { recursive: true });
  fs.writeFileSync(
    path.join(runtimeDir, "version.txt"),
    [
      `version=${version}`,
      "rootfs-sha256=rootfs-sha",
      "kernel-sha256=kernel-sha",
      "initrd-sha256=initrd-sha",
      "guest-agent-sha256=current-guest-agent-sha",
      "egress-proxy-sha256=egress-sha",
      "container-stack-sha256=container-stack-sha",
      "",
    ].join("\n"),
    "utf8",
  );

  const manifest = {
    version: 1,
    providers: [],
    runtimes: [
      {
        id: "avf-linux-guest",
        version,
        os: "macos",
        arch: "aarch64",
        sha256: "rootfs-sha",
        root,
        bin: "rootfs.raw",
      },
    ],
    images: [],
  };
  const lock = {
    version: 2,
    components: [
      {
        kind: "runtime",
        id: "avf-linux-guest",
        os: "macos",
        arch: "aarch64",
        variant: "default",
        version,
        helpers: {
          kernel: { uri: `https://example.invalid/${version}/macos/aarch64/sha256-kernel/kernel`, sha256: "kernel-sha" },
          initrd: { uri: `https://example.invalid/${version}/macos/aarch64/sha256-initrd/initrd`, sha256: "initrd-sha" },
          "guest-agent": { uri: `https://example.invalid/${version}/macos/aarch64/sha256-guest/guest-agent`, sha256: "stale-guest-agent-sha" },
          "egress-proxy": { uri: `https://example.invalid/${version}/macos/aarch64/sha256-egress/egress-proxy`, sha256: "egress-sha" },
          "container-stack": { uri: `https://example.invalid/${version}/macos/aarch64/sha256-stack/container-stack.tar.gz`, sha256: "container-stack-sha" },
        },
        sources: [
          { source_type: "ci", uri: `https://example.invalid/${version}/macos/aarch64/sha256-rootfs/rootfs.raw.zst`, sha256: "rootfs-zst-sha" },
        ],
      },
    ],
  };

  const result = validateAvfRuntimeFreshness({ runtimeDir, bundleDir, manifest, lock });
  assert.equal(result.ok, false);
  assert.ok(result.errors.some((entry) => entry.includes("guest-agent sha drift")));

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});

test("validateAvfRuntimeFreshness allows managed shipped-app metadata without bundled runtime payload", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-avf-runtime-freshness-managed-"));
  const runtimeDir = path.join(tmpRoot, "runtime");
  const bundleDir = path.join(tmpRoot, "bundles");
  const version = "ubuntu-noble-arm64-f024c70f1f22";
  const root = expectedBundleRoot(version);
  fs.mkdirSync(runtimeDir, { recursive: true });
  fs.mkdirSync(bundleDir, { recursive: true });
  fs.writeFileSync(
    path.join(runtimeDir, "version.txt"),
    [
      `version=${version}`,
      "rootfs-sha256=rootfs-sha",
      "kernel-sha256=kernel-sha",
      "initrd-sha256=initrd-sha",
      "guest-agent-sha256=guest-agent-sha",
      "egress-proxy-sha256=egress-sha",
      "container-stack-sha256=container-stack-sha",
      "",
    ].join("\n"),
    "utf8",
  );

  const manifest = {
    version: 1,
    providers: [],
    runtimes: [
      {
        id: "avf-linux-guest",
        version,
        os: "macos",
        arch: "aarch64",
        sha256: "rootfs-sha",
        root,
        bin: "rootfs.raw",
      },
    ],
    images: [],
  };
  const lock = {
    version: 2,
    components: [
      {
        kind: "runtime",
        id: "avf-linux-guest",
        os: "macos",
        arch: "aarch64",
        variant: "default",
        version,
        helpers: {
          kernel: { uri: `https://example.invalid/${version}/macos/aarch64/sha256-kernel/kernel`, sha256: "kernel-sha" },
          initrd: { uri: `https://example.invalid/${version}/macos/aarch64/sha256-initrd/initrd`, sha256: "initrd-sha" },
          "guest-agent": { uri: `https://example.invalid/${version}/macos/aarch64/sha256-guest/guest-agent`, sha256: "guest-agent-sha" },
          "egress-proxy": { uri: `https://example.invalid/${version}/macos/aarch64/sha256-egress/egress-proxy`, sha256: "egress-sha" },
          "container-stack": { uri: `https://example.invalid/${version}/macos/aarch64/sha256-stack/container-stack.tar.gz`, sha256: "container-stack-sha" },
        },
        sources: [
          { source_type: "ci", uri: `https://example.invalid/${version}/macos/aarch64/sha256-rootfs/rootfs.raw.zst`, sha256: "rootfs-zst-sha" },
        ],
      },
    ],
  };

  const result = validateAvfRuntimeFreshness({
    runtimeDir,
    bundleDir,
    manifest,
    lock,
    allowManagedRuntime: true,
  });
  assert.equal(result.ok, true, result.errors.join("\n"));

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});
