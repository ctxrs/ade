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

test("linux ctx-mcp runtime bundling can be disabled for host-only local prep", () => {
  assert.equal(
    __desktopSyncResourcesTestHooks.shouldBundleLinuxCtxMcpRuntime("darwin", {
      CTX_BUNDLE_LINUX_CTX_MCP_RUNTIME: "0",
    }),
    false,
  );
  assert.equal(
    __desktopSyncResourcesTestHooks.shouldBundleLinuxCtxMcpRuntime("linux", {
      CTX_BUNDLE_LINUX_CTX_MCP_RUNTIME: "0",
    }),
    false,
  );
});

test("desktop sync resources treats debug packaging as packaged artifact identity", () => {
  assert.equal(
    __desktopSyncResourcesTestHooks.resolveArtifactIdentityMode({}),
    "packaged",
  );
});

test("desktop sync resources maps explicit Rust target triples to bundle os/arch", () => {
  assert.deepEqual(
    __desktopSyncResourcesTestHooks.resolvePrimaryBundleTargetEnv({
      env: { CARGO_BUILD_TARGET: "aarch64-apple-darwin" },
    }),
    { CTX_BUNDLE_OS: "macos", CTX_BUNDLE_ARCH: "aarch64" },
  );
  assert.deepEqual(
    __desktopSyncResourcesTestHooks.resolvePrimaryBundleTargetEnv({
      env: { CARGO_BUILD_TARGET: "x86_64-apple-darwin" },
    }),
    { CTX_BUNDLE_OS: "macos", CTX_BUNDLE_ARCH: "x86_64" },
  );
});

test("desktop sync resources strips provider release channel from bundled manifest", () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-bundle-provider-matrix-"));
  const sourceDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-source-provider-matrix-"));
  try {
    const matrixPath = path.join(sourceDir, "provider_matrix.json");
    fs.writeFileSync(
      matrixPath,
      `${JSON.stringify({
        version: 3,
        release_channel: "canary",
        providers: [{ id: "codex", display_name: "Codex" }],
      }, null, 2)}\n`,
      "utf8",
    );

    const targetPath = __desktopSyncResourcesTestHooks.writeBundledProviderManifest(bundleDir, {
      CTX_BUNDLE_MATRIX_JSON: matrixPath,
    });
    const bundled = JSON.parse(fs.readFileSync(targetPath, "utf8"));

    assert.equal(Object.prototype.hasOwnProperty.call(bundled, "release_channel"), false);
    assert.deepEqual(bundled.providers, [{ id: "codex", display_name: "Codex" }]);
  } finally {
    fs.rmSync(bundleDir, { recursive: true, force: true });
    fs.rmSync(sourceDir, { recursive: true, force: true });
  }
});

test("linux ctx-mcp runtime bundling follows the target app arch instead of the build host arch", () => {
  assert.equal(
    __desktopSyncResourcesTestHooks.resolveLinuxCtxMcpRuntimeArch({
      env: { CARGO_BUILD_TARGET: "aarch64-apple-darwin" },
      fallbackArch: "x86_64",
    }),
    "aarch64",
  );
  assert.equal(
    __desktopSyncResourcesTestHooks.resolveLinuxCtxMcpRuntimeArch({
      env: { CARGO_BUILD_TARGET: "x86_64-apple-darwin" },
      fallbackArch: "aarch64",
    }),
    "x86_64",
  );
});

test("desktop sync resources reads the default container image from the sandbox container runtime crate", () => {
  assert.equal(
    __desktopSyncResourcesTestHooks.readDefaultContainerImage(),
    "ghcr.io/ctxrs/ctx-harness:ubuntu-24.04",
  );
});

test("host linux ctx-mcp runtime bundling stages a prepared executable without containers", () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-host-ctx-mcp-bundle-"));
  const sourceDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-host-ctx-mcp-bin-"));
  try {
    fs.writeFileSync(
      path.join(bundleDir, "manifest.json"),
      JSON.stringify({ version: 1, providers: [], runtimes: [], images: [], daemons: [] }, null, 2),
      "utf8",
    );
    const sourcePath = path.join(sourceDir, "ctx-mcp");
    fs.writeFileSync(sourcePath, "#!/bin/sh\nexit 0\n", "utf8");
    fs.chmodSync(sourcePath, 0o755);

    __desktopSyncResourcesTestHooks.bundleHostLinuxCtxMcpRuntime(bundleDir, {
      arch: "x86_64",
      hostOs: "linux",
      runtimeVersion: "9.8.7",
      sourcePath,
    });

    const runtimeRel = "runtimes/ctx-mcp/linux/x86_64/9.8.7";
    const runtimePath = path.join(bundleDir, runtimeRel, "ctx-mcp");
    fs.accessSync(runtimePath, fs.constants.X_OK);
    const manifest = JSON.parse(fs.readFileSync(path.join(bundleDir, "manifest.json"), "utf8"));
    assert.deepEqual(manifest.runtimes, [{
      id: "ctx-mcp",
      version: "9.8.7",
      os: "linux",
      arch: "x86_64",
      sha256: manifest.runtimes[0].sha256,
      root: runtimeRel,
      bin: "ctx-mcp",
    }]);
    assert.match(manifest.runtimes[0].sha256, /^[0-9a-f]{64}$/);
  } finally {
    fs.rmSync(bundleDir, { recursive: true, force: true });
    fs.rmSync(sourceDir, { recursive: true, force: true });
  }
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
