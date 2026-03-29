const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  resolveBundledRuntimeIds,
  stageAvfLinuxGuestRuntime,
  __desktopSyncResourcesTestHooks,
} = require("./desktop_sync_resources.cjs");

const hostManifestOs = process.platform === "darwin" ? "macos" : process.platform === "win32" ? "windows" : "linux";
const hostManifestArch = process.arch === "arm64" ? "aarch64" : process.arch === "x64" ? "x86_64" : process.arch;
const avfManifestOs = "macos";
const avfManifestArch = "aarch64";

test("resolveBundledRuntimeIds dedupes and sorts bundle runtime identifiers", () => {
  assert.deepEqual(
    resolveBundledRuntimeIds(["python", "avf-linux-guest", "python", "zeta"]),
    ["avf-linux-guest", "python", "zeta"],
  );
});

test("linux ctx-mcp runtime bundling stays enabled on desktop platforms with Linux sandboxes", () => {
  assert.equal(__desktopSyncResourcesTestHooks.shouldBundleLinuxCtxMcpRuntime("darwin"), true);
  assert.equal(__desktopSyncResourcesTestHooks.shouldBundleLinuxCtxMcpRuntime("linux"), true);
  assert.equal(__desktopSyncResourcesTestHooks.shouldBundleLinuxCtxMcpRuntime("win32"), false);
});

test("stageAvfLinuxGuestRuntime leaves the bundle untouched when no guest artifact is present", () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-bundle-"));
  const staged = stageAvfLinuxGuestRuntime(bundleDir);
  assert.equal(staged, null);
  assert.equal(fs.existsSync(path.join(bundleDir, "runtimes", "avf-linux-guest")), false);
});

test("thin bundle parity rejects unresolved managed AVF runtime metadata from runtime lock", () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-bundle-runtime-lock-"));
  try {
    fs.writeFileSync(
      path.join(bundleDir, "manifest.json"),
      JSON.stringify({ version: 1, providers: [], runtimes: [], images: [], daemons: [] }, null, 2),
      "utf8",
    );
    fs.writeFileSync(
      path.join(bundleDir, "runtime_lock.v2.json"),
      JSON.stringify(
        {
          version: 2,
          required: {
            provider_ids: [],
            runtime_ids: ["avf-linux-guest"],
            image_ids: [],
            machine_cache_ids: [],
            targets: {
              provider: [],
              runtime: ["macos/aarch64"],
              image: [],
              machine_cache: [],
            },
          },
          components: [
            {
              kind: "runtime",
              id: "avf-linux-guest",
              os: avfManifestOs,
              arch: avfManifestArch,
              variant: "default",
              version: "test",
              sources: [{ source_type: "ci", uri: "locked://runtime", sha256: "0".repeat(64) }],
              helpers: {
                kernel: { uri: "locked://kernel", sha256: "1".repeat(64) },
                initrd: { uri: "locked://initrd", sha256: "2".repeat(64) },
                "guest-agent": { uri: "locked://guest-agent", sha256: "3".repeat(64) },
                "egress-proxy": { uri: "locked://egress-proxy", sha256: "4".repeat(64) },
              },
            },
          ],
        },
        null,
        2,
      ),
      "utf8",
    );

    assert.throws(
      () => {
        __desktopSyncResourcesTestHooks.assertRuntimeTargetsAvailable(bundleDir, "avf-linux-guest", [
          { os: avfManifestOs, arch: avfManifestArch },
        ]);
      },
      /bundle\/runtime lock missing avf-linux-guest runtime targets/,
    );
  } finally {
    fs.rmSync(bundleDir, { recursive: true, force: true });
  }
});

test("thin bundle parity rejects AVF runtime metadata missing helper payloads", () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-bundle-runtime-lock-"));
  try {
    fs.writeFileSync(
      path.join(bundleDir, "manifest.json"),
      JSON.stringify({ version: 1, providers: [], runtimes: [], images: [], daemons: [] }, null, 2),
      "utf8",
    );
    fs.writeFileSync(
      path.join(bundleDir, "runtime_lock.v2.json"),
      JSON.stringify(
        {
          version: 2,
          required: {
            provider_ids: [],
            runtime_ids: ["avf-linux-guest"],
            image_ids: [],
            machine_cache_ids: [],
            targets: {
              provider: [],
              runtime: ["macos/aarch64"],
              image: [],
              machine_cache: [],
            },
          },
          components: [
            {
              kind: "runtime",
              id: "avf-linux-guest",
              os: avfManifestOs,
              arch: avfManifestArch,
              variant: "default",
              version: "test",
              sources: [{ source_type: "ci", uri: "locked://runtime", sha256: "0".repeat(64) }],
              helpers: {
                kernel: { uri: "locked://kernel", sha256: "1".repeat(64) },
                initrd: { uri: "locked://initrd", sha256: "2".repeat(64) },
                "egress-proxy": { uri: "locked://egress-proxy", sha256: "4".repeat(64) },
              },
            },
          ],
        },
        null,
        2,
      ),
      "utf8",
    );

    assert.throws(
      () =>
        __desktopSyncResourcesTestHooks.assertRuntimeTargetsAvailable(bundleDir, "avf-linux-guest", [
          { os: hostManifestOs, arch: hostManifestArch },
        ]),
      /bundle\/runtime lock missing avf-linux-guest runtime targets/,
    );
  } finally {
    fs.rmSync(bundleDir, { recursive: true, force: true });
  }
});
