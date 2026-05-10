const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const modulePath = require.resolve("./wdio.conf.cjs");

const desktopOs = () => {
  if (process.platform === "darwin") return "macos";
  if (process.platform === "win32") return "windows";
  return "linux";
};

const desktopArch = () => {
  if (process.arch === "arm64") return "aarch64";
  if (process.arch === "x64") return "x86_64";
  return process.arch;
};

const linuxImageTargetsForFixture = ["aarch64", "x86_64"];

const withEnv = async (overrides, fn) => {
  const previous = new Map();
  for (const key of Object.keys(overrides)) {
    previous.set(key, process.env[key]);
    const value = overrides[key];
    if (value === null) {
      delete process.env[key];
    } else {
      process.env[key] = value;
    }
  }

  delete require.cache[modulePath];
  try {
    return await fn(require("./wdio.conf.cjs"));
  } finally {
    delete require.cache[modulePath];
    for (const [key, value] of previous.entries()) {
      if (typeof value === "undefined") {
        delete process.env[key];
      } else {
        process.env[key] = value;
      }
    }
  }
};

const resolveFixtureHost = ({ hostOs = desktopOs(), hostArch = desktopArch() } = {}) => ({
  hostOs,
  hostArch,
});

const managedRuntimeShaByFilename = {
  "rootfs.raw.zst": "0".repeat(64),
  kernel: "1".repeat(64),
  initrd: "2".repeat(64),
  "guest-agent": "3".repeat(64),
  "egress-proxy": "4".repeat(64),
  "container-stack.tar.gz": "5".repeat(64),
};

const makeManagedRuntimeSpawnSync = (runtimeDir) => (cmd, args) => {
  if (cmd === "curl") {
    const outputPath = args[4];
    fs.mkdirSync(path.dirname(outputPath), { recursive: true });
    fs.writeFileSync(outputPath, `${path.basename(outputPath)}\n`, "utf8");
    return { status: 0, stdout: "", stderr: "" };
  }
  if (cmd === "shasum") {
    const filePath = args[2];
    const basename = path.basename(filePath);
    const sha = managedRuntimeShaByFilename[basename];
    assert.ok(sha, `unexpected sha request for ${basename}`);
    return { status: 0, stdout: `${sha}  ${filePath}\n`, stderr: "" };
  }
  if (cmd === "zstd") {
    const outputPath = args[4];
    fs.writeFileSync(outputPath, "rootfs\n", "utf8");
    return { status: 0, stdout: "", stderr: "" };
  }
  assert.fail(`unexpected command ${cmd} ${args.join(" ")}`);
};

const writeAvfRuntimeBundle = (bundleDir, options = {}) => {
  const {
    includeGuestAgent = true,
    includeContainerStack = true,
    version = "test",
    ...hostOverride
  } = options;
  const { hostOs, hostArch } = resolveFixtureHost(hostOverride);
  const runtimeRootRel = path.join("runtimes", "avf-linux-guest", hostOs, hostArch, "dev");
  const runtimeRoot = path.join(bundleDir, runtimeRootRel);
  fs.mkdirSync(path.join(runtimeRoot, "helpers"), { recursive: true });
  fs.writeFileSync(path.join(runtimeRoot, "rootfs.raw"), "rootfs\n", "utf8");
  fs.writeFileSync(path.join(runtimeRoot, "helpers", "kernel"), "kernel\n", "utf8");
  fs.writeFileSync(path.join(runtimeRoot, "helpers", "initrd"), "initrd\n", "utf8");
  if (includeGuestAgent) {
    fs.writeFileSync(path.join(runtimeRoot, "helpers", "guest-agent"), "guest-agent\n", "utf8");
  }
  fs.writeFileSync(path.join(runtimeRoot, "helpers", "egress-proxy"), "egress-proxy\n", "utf8");
  if (includeContainerStack) {
    fs.writeFileSync(path.join(runtimeRoot, "helpers", "container-stack.tar.gz"), "container-stack\n", "utf8");
  }
  fs.writeFileSync(path.join(runtimeRoot, "version.txt"), `version=${version}\n`, "utf8");
  fs.writeFileSync(
    path.join(bundleDir, "manifest.json"),
    JSON.stringify(
      {
        version: 1,
        providers: [],
        runtimes: [
          {
            id: "avf-linux-guest",
            os: hostOs,
            arch: hostArch,
            root: runtimeRootRel,
            bin: "rootfs.raw",
          },
        ],
        images: [],
        daemons: [],
      },
      null,
      2,
    ),
    "utf8",
  );
  fs.writeFileSync(
    path.join(bundleDir, "runtime_lock.v2.json"),
    JSON.stringify(
      {
        version: 2,
        profiles: {
          parity: { allowed_source_types: ["ci", "vendor"] },
        },
        required: {
          provider_ids: [],
          runtime_ids: ["avf-linux-guest"],
          image_ids: ["ctx-harness"],
          machine_cache_ids: [],
          targets: {
            provider: [],
            runtime: ["macos/host"],
            image: ["linux/host"],
            machine_cache: [],
          },
        },
        components: [
          {
            kind: "runtime",
            id: "avf-linux-guest",
            os: hostOs,
            arch: hostArch,
            variant: "default",
            version,
            sources: [{ source_type: "ci", uri: "locked://runtime", sha256: "0".repeat(64) }],
            helpers: {
              kernel: { uri: "locked://kernel", sha256: "1".repeat(64) },
              initrd: { uri: "locked://initrd", sha256: "2".repeat(64) },
              "guest-agent": { uri: "locked://guest-agent", sha256: "3".repeat(64) },
              "egress-proxy": { uri: "locked://egress-proxy", sha256: "4".repeat(64) },
              "container-stack": { uri: "locked://container-stack", sha256: "5".repeat(64) },
            },
          },
          ...linuxImageTargetsForFixture.map((arch, index) => ({
            kind: "image",
            id: "ctx-harness",
            os: "linux",
            arch,
            variant: "default",
            version: "test",
            sources: [{
              source_type: "ci",
              uri: `locked://image-${arch}`,
              sha256: String(index + 6).repeat(64),
            }],
          })),
        ],
      },
      null,
      2,
    ),
    "utf8",
  );
};

const writeThinBundleManifestAndRuntimeLock = (bundleDir, options = {}) => {
  const {
    includeGuestAgent = true,
    includeContainerStack = true,
    version = "test",
    ...hostOverride
  } = options;
  const { hostOs, hostArch } = resolveFixtureHost(hostOverride);
  fs.writeFileSync(
    path.join(bundleDir, "manifest.json"),
    JSON.stringify({ version: 1, providers: [], runtimes: [], images: [], daemons: [] }, null, 2),
    "utf8",
  );
  const helpers = {
    kernel: { uri: "locked://kernel", sha256: "1".repeat(64) },
    initrd: { uri: "locked://initrd", sha256: "2".repeat(64) },
    "egress-proxy": { uri: "locked://egress-proxy", sha256: "4".repeat(64) },
  };
  if (includeGuestAgent) {
    helpers["guest-agent"] = { uri: "locked://guest-agent", sha256: "3".repeat(64) };
  }
  if (includeContainerStack) {
    helpers["container-stack"] = { uri: "locked://container-stack", sha256: "5".repeat(64) };
  }
  fs.writeFileSync(
    path.join(bundleDir, "runtime_lock.v2.json"),
    JSON.stringify(
      {
        version: 2,
        profiles: {
          parity: { allowed_source_types: ["ci", "vendor"] },
        },
        required: {
          provider_ids: [],
          runtime_ids: ["avf-linux-guest"],
          image_ids: ["ctx-harness"],
          machine_cache_ids: [],
          targets: {
            provider: [],
            runtime: ["macos/host"],
            image: ["linux/host"],
            machine_cache: [],
          },
        },
        components: [
          {
            kind: "runtime",
            id: "avf-linux-guest",
            os: hostOs,
            arch: hostArch,
            variant: "default",
            version,
            sources: [{ source_type: "ci", uri: "locked://runtime", sha256: "0".repeat(64) }],
            helpers,
          },
          ...linuxImageTargetsForFixture.map((arch, index) => ({
            kind: "image",
            id: "ctx-harness",
            os: "linux",
            arch,
            variant: "default",
            version: "test",
            sources: [{
              source_type: "ci",
              uri: `locked://image-${arch}`,
              sha256: String(index + 6).repeat(64),
            }],
          })),
        ],
      },
      null,
      2,
    ),
    "utf8",
  );
};

test("wdio AVF container preflight accepts a complete guest runtime payload", async () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-avf-assets-"));
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-state-"));
  try {
    writeAvfRuntimeBundle(bundleDir, { hostOs: "macos", hostArch: "aarch64" });
    await withEnv(
      {
        CTX_BUNDLE_DIR: bundleDir,
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
      },
      (mod) => {
        assert.doesNotThrow(() => {
          mod.__desktopAutomationConfigTestHooks.ensureBundledContainerAssets({
            platform: "darwin",
            arch: "arm64",
          });
        });
      },
    );
  } finally {
    fs.rmSync(bundleDir, { recursive: true, force: true });
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});

test("wdio AVF container preflight rejects a bundle missing the guest agent helper on macOS", async () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-avf-assets-"));
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-state-"));
  try {
    writeAvfRuntimeBundle(bundleDir, {
      includeGuestAgent: false,
      hostOs: "macos",
      hostArch: "aarch64",
    });
    await withEnv(
      {
        CTX_BUNDLE_DIR: bundleDir,
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
      },
      (mod) => {
        assert.throws(
          () =>
            mod.__desktopAutomationConfigTestHooks.ensureBundledContainerAssets({
              platform: "darwin",
              arch: "arm64",
            }),
          /guest-agent/,
        );
      },
    );
  } finally {
    fs.rmSync(bundleDir, { recursive: true, force: true });
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});

test("wdio AVF container preflight accepts a thin bundle with managed AVF/image sources", async () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-avf-assets-"));
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-state-"));
  try {
    writeThinBundleManifestAndRuntimeLock(bundleDir, { hostOs: "macos", hostArch: "aarch64" });
    await withEnv(
      {
        CTX_BUNDLE_DIR: bundleDir,
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
      },
      (mod) => {
        assert.doesNotThrow(() => {
          mod.__desktopAutomationConfigTestHooks.ensureBundledContainerAssets({
            platform: "darwin",
            arch: "arm64",
          });
        });
      },
    );
  } finally {
    fs.rmSync(bundleDir, { recursive: true, force: true });
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});

test("wdio container preflight does not require managed AVF runtime metadata on macOS x64", async () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-avf-assets-"));
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-state-"));
  try {
    writeThinBundleManifestAndRuntimeLock(bundleDir, { hostOs: "macos", hostArch: "aarch64" });
    await withEnv(
      {
        CTX_BUNDLE_DIR: bundleDir,
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
      },
      (mod) => {
        assert.doesNotThrow(() => {
          mod.__desktopAutomationConfigTestHooks.ensureBundledContainerAssets({
            platform: "darwin",
            arch: "x64",
          });
        });
      },
    );
  } finally {
    fs.rmSync(bundleDir, { recursive: true, force: true });
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});

test("wdio AVF container preflight rejects a thin bundle missing managed AVF helper metadata on macOS", async () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-avf-assets-"));
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-state-"));
  try {
    writeThinBundleManifestAndRuntimeLock(bundleDir, {
      includeGuestAgent: false,
      hostOs: "macos",
      hostArch: "aarch64",
    });
    await withEnv(
      {
        CTX_BUNDLE_DIR: bundleDir,
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
      },
      (mod) => {
        assert.throws(
          () =>
            mod.__desktopAutomationConfigTestHooks.ensureBundledContainerAssets({
              platform: "darwin",
              arch: "arm64",
            }),
          /guest-agent/,
        );
      },
    );
  } finally {
    fs.rmSync(bundleDir, { recursive: true, force: true });
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});

test("wdio AVF container preflight accepts managed runtime metadata when local payload is intentionally skipped", async () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-avf-assets-"));
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-state-"));
  try {
    writeThinBundleManifestAndRuntimeLock(bundleDir, { hostOs: "macos", hostArch: "aarch64" });
    const manifestPath = path.join(bundleDir, "manifest.json");
    const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
    manifest.runtimes = [
      {
        id: "avf-linux-guest",
        os: "macos",
        arch: "aarch64",
        root: "runtimes/avf-linux-guest/macos/aarch64/test",
        bin: "rootfs.raw",
      },
    ];
    fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2));
    await withEnv(
      {
        CTX_BUNDLE_DIR: bundleDir,
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
        CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD: "1",
      },
      (mod) => {
        assert.doesNotThrow(() => {
          mod.__desktopAutomationConfigTestHooks.ensureBundledContainerAssets({
            platform: "darwin",
            arch: "arm64",
          });
        });
      },
    );
  } finally {
    fs.rmSync(bundleDir, { recursive: true, force: true });
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});

test("wdio automation AVF runtime prep reuses an existing prepared runtime directory", async () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-bundle-"));
  const runtimeDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-avf-runtime-"));
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-state-"));
  const logs = [];
  try {
    writeThinBundleManifestAndRuntimeLock(bundleDir, { hostOs: "macos", hostArch: "aarch64", version: "test" });
    fs.mkdirSync(path.join(runtimeDir, "helpers"), { recursive: true });
    fs.writeFileSync(path.join(runtimeDir, "rootfs.raw"), "rootfs\n", "utf8");
    fs.writeFileSync(path.join(runtimeDir, "helpers", "kernel"), "kernel\n", "utf8");
    fs.writeFileSync(path.join(runtimeDir, "helpers", "initrd"), "initrd\n", "utf8");
    fs.writeFileSync(path.join(runtimeDir, "helpers", "guest-agent"), "guest-agent\n", "utf8");
    fs.writeFileSync(path.join(runtimeDir, "helpers", "egress-proxy"), "egress-proxy\n", "utf8");
    fs.writeFileSync(path.join(runtimeDir, "helpers", "container-stack.tar.gz"), "container-stack\n", "utf8");
    fs.writeFileSync(path.join(runtimeDir, "version.txt"), "version=test\n", "utf8");
    await withEnv(
      {
        CTX_BUNDLE_DIR: bundleDir,
        CTX_AVF_LINUX_GUEST_RUNTIME_DIR: runtimeDir,
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
      },
      (mod) => {
        let spawnCalled = false;
        const resolved = mod.__desktopAutomationConfigTestHooks.ensureAutomationAvfLinuxGuestRuntime({
          platform: "darwin",
          arch: "arm64",
          runsContainerScenarios: true,
          env: process.env,
          log: (line) => logs.push(line),
          spawnSyncImpl: () => {
            spawnCalled = true;
            return { status: 0, stdout: "", stderr: "" };
          },
        });
        assert.equal(resolved, runtimeDir);
        assert.equal(process.env.CTX_AVF_LINUX_GUEST_RUNTIME_DIR, runtimeDir);
        assert.equal(spawnCalled, false);
      },
    );
    assert.ok(logs.some((line) => line.includes("CTX_AVF_LINUX_GUEST_RUNTIME_DIR")));
  } finally {
    fs.rmSync(bundleDir, { recursive: true, force: true });
    fs.rmSync(runtimeDir, { recursive: true, force: true });
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});

test("wdio automation AVF runtime prep refreshes a stale prepared runtime directory when the version drifts", async () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-bundle-"));
  const runtimeDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-avf-runtime-"));
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-state-"));
  const logs = [];
  try {
    writeThinBundleManifestAndRuntimeLock(bundleDir, {
      hostOs: "macos",
      hostArch: "aarch64",
      version: "expected-version",
    });
    fs.mkdirSync(path.join(runtimeDir, "helpers"), { recursive: true });
    fs.writeFileSync(path.join(runtimeDir, "rootfs.raw"), "rootfs\n", "utf8");
    fs.writeFileSync(path.join(runtimeDir, "helpers", "kernel"), "kernel\n", "utf8");
    fs.writeFileSync(path.join(runtimeDir, "helpers", "initrd"), "initrd\n", "utf8");
    fs.writeFileSync(path.join(runtimeDir, "helpers", "guest-agent"), "guest-agent\n", "utf8");
    fs.writeFileSync(path.join(runtimeDir, "helpers", "egress-proxy"), "egress-proxy\n", "utf8");
    fs.writeFileSync(path.join(runtimeDir, "helpers", "container-stack.tar.gz"), "container-stack\n", "utf8");
    fs.writeFileSync(path.join(runtimeDir, "version.txt"), "version=stale-version\n", "utf8");
    await withEnv(
      {
        CTX_BUNDLE_DIR: bundleDir,
        CTX_AVF_LINUX_GUEST_RUNTIME_DIR: runtimeDir,
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
      },
      (mod) => {
        const commands = [];
        const resolved = mod.__desktopAutomationConfigTestHooks.ensureAutomationAvfLinuxGuestRuntime({
          platform: "darwin",
          arch: "arm64",
          runsContainerScenarios: true,
          env: process.env,
          log: (line) => logs.push(line),
          spawnSyncImpl: (cmd, args, options) => {
            commands.push([cmd, ...args]);
            return makeManagedRuntimeSpawnSync(runtimeDir)(cmd, args, options);
          },
        });
        assert.equal(resolved, runtimeDir);
        assert.equal(process.env.CTX_AVF_LINUX_GUEST_RUNTIME_DIR, runtimeDir);
        assert.ok(commands.some((entry) => entry[0] === "curl"));
        assert.ok(commands.some((entry) => entry[0] === "zstd"));
        assert.equal(
          mod.__desktopAutomationConfigTestHooks.readAvfGuestRuntimeVersion(runtimeDir),
          "expected-version",
        );
      },
    );
    assert.ok(logs.some((line) => line.includes("refreshing stale AVF Linux guest runtime")));
  } finally {
    fs.rmSync(bundleDir, { recursive: true, force: true });
    fs.rmSync(runtimeDir, { recursive: true, force: true });
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});

test("wdio automation AVF runtime prep prepares and exports a missing runtime directory", async () => {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-avf-runtime-parent-"));
  const runtimeDir = path.join(tempRoot, "runtime");
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-bundle-"));
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-state-"));
  try {
    writeThinBundleManifestAndRuntimeLock(bundleDir, {
      hostOs: "macos",
      hostArch: "aarch64",
      version: "expected-version",
    });
    await withEnv(
      {
        CTX_BUNDLE_DIR: bundleDir,
        CTX_AVF_LINUX_GUEST_RUNTIME_DIR: null,
        CTX_AUTOMATION_AVF_LINUX_GUEST_RUNTIME_DIR: runtimeDir,
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
      },
      (mod) => {
        const resolved = mod.__desktopAutomationConfigTestHooks.ensureAutomationAvfLinuxGuestRuntime({
          platform: "darwin",
          arch: "arm64",
          runsContainerScenarios: true,
          env: process.env,
          spawnSyncImpl: makeManagedRuntimeSpawnSync(runtimeDir),
          log: () => {},
        });
        assert.equal(resolved, runtimeDir);
        assert.equal(process.env.CTX_AVF_LINUX_GUEST_RUNTIME_DIR, runtimeDir);
        assert.equal(
          mod.__desktopAutomationConfigTestHooks.readAvfGuestRuntimeVersion(runtimeDir),
          "expected-version",
        );
      },
    );
  } finally {
    fs.rmSync(bundleDir, { recursive: true, force: true });
    fs.rmSync(tempRoot, { recursive: true, force: true });
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});

test("scoped app sweep matcher accepts canonical bundle paths for a symlinked app path", async () => {
  const appRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-app-real-"));
  const linkParent = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-app-link-"));
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-state-"));
  const realAppPath = path.join(appRoot, "ctx.app");
  const linkedAppPath = path.join(linkParent, "ctx.app");
  const realMacOsDir = path.join(realAppPath, "Contents", "MacOS");
  const realBinDir = path.join(realAppPath, "Contents", "Resources", "bin");
  try {
    fs.mkdirSync(realMacOsDir, { recursive: true });
    fs.mkdirSync(realBinDir, { recursive: true });
    fs.writeFileSync(path.join(realMacOsDir, "ctx"), "", "utf8");
    fs.symlinkSync(realAppPath, linkedAppPath, "dir");

    await withEnv(
      {
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
      },
      (mod) => {
        const hooks = mod.__desktopAutomationConfigTestHooks;
        const realExecutablePath = fs.realpathSync(path.join(linkedAppPath, "Contents", "MacOS", "ctx"));
        const realHelperPath = path.join(fs.realpathSync(path.join(linkedAppPath, "Contents", "Resources", "bin")), "ctx-avf-linux-helper-aarch64-apple-darwin");
        assert.equal(
          hooks.commandMatchesScopedAppProcess(`${realExecutablePath} --automation`, linkedAppPath),
          true,
        );
        assert.equal(
          hooks.commandMatchesScopedAppProcess(`${realHelperPath} run-workspace-vm /tmp/ctx-data`, linkedAppPath),
          true,
        );
      },
    );
  } finally {
    fs.rmSync(appRoot, { recursive: true, force: true });
    fs.rmSync(linkParent, { recursive: true, force: true });
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});

test("shipped-app process sweep also matches stale source-built desktop binaries", async () => {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-process-sweep-"));
  const targetDir = path.join(tempRoot, "target");
  const shippedDir = path.join(tempRoot, "appimage", "squashfs-root");
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-state-"));
  const shippedApp = path.join(shippedDir, "AppRun");
  const shippedInnerApp = path.join(shippedDir, "usr", "bin", process.platform === "win32" ? "ctx.exe" : "ctx");
  const shippedHelper = path.join(shippedDir, "usr", "lib", "ctx", "bin", "ctx-mcp");
  const sourceDebugApp = path.join(targetDir, "debug", process.platform === "win32" ? "ctx.exe" : "ctx");
  try {
    fs.mkdirSync(path.dirname(shippedApp), { recursive: true });
    fs.mkdirSync(path.dirname(shippedInnerApp), { recursive: true });
    fs.mkdirSync(path.dirname(shippedHelper), { recursive: true });
    fs.mkdirSync(path.dirname(sourceDebugApp), { recursive: true });
    fs.writeFileSync(shippedApp, "#!/bin/sh\nexit 0\n", { mode: 0o700 });
    fs.writeFileSync(shippedInnerApp, "#!/bin/sh\nexit 0\n", { mode: 0o700 });
    fs.writeFileSync(shippedHelper, "#!/bin/sh\nexit 0\n", { mode: 0o700 });
    fs.writeFileSync(sourceDebugApp, "#!/bin/sh\nexit 0\n", { mode: 0o700 });

    await withEnv(
      {
        CARGO_TARGET_DIR: targetDir,
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
        CTX_AUTOMATION_SHIPPED_APP: "1",
        CTX_DESKTOP_APP_PATH: shippedApp,
      },
      (mod) => {
        const hooks = mod.__desktopAutomationConfigTestHooks;
        assert.ok(hooks.collectAutomationAppProcessSweepPaths().includes(shippedApp));
        assert.ok(hooks.collectAutomationAppProcessSweepPaths().includes(shippedInnerApp));
        assert.ok(hooks.collectAutomationAppProcessSweepPaths().includes(sourceDebugApp));
        assert.equal(
          hooks.commandMatchesAutomationAppProcess(`${shippedInnerApp} --automation`),
          true,
        );
        assert.equal(
          hooks.commandMatchesAutomationAppProcess(`${shippedHelper} mcp`),
          true,
        );
        assert.equal(
          hooks.commandMatchesAutomationAppProcess(`${sourceDebugApp} --automation`),
          true,
        );
        assert.equal(
          hooks.commandMatchesAutomationAppProcess("/usr/bin/ctx --unrelated"),
          false,
        );
      },
    );
  } finally {
    fs.rmSync(tempRoot, { recursive: true, force: true });
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});
