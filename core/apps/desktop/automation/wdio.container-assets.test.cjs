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

const writeAvfRuntimeBundle = (bundleDir, options = {}) => {
  const { includeGuestAgent = true, includeContainerStack = true, ...hostOverride } = options;
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
            version: "test",
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
  const { includeGuestAgent = true, includeContainerStack = true, ...hostOverride } = options;
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
            version: "test",
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

test("wdio automation AVF runtime prep reuses an existing prepared runtime directory", async () => {
  const runtimeDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-avf-runtime-"));
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-state-"));
  const logs = [];
  try {
    fs.mkdirSync(path.join(runtimeDir, "helpers"), { recursive: true });
    fs.writeFileSync(path.join(runtimeDir, "rootfs.raw"), "rootfs\n", "utf8");
    fs.writeFileSync(path.join(runtimeDir, "helpers", "kernel"), "kernel\n", "utf8");
    fs.writeFileSync(path.join(runtimeDir, "helpers", "initrd"), "initrd\n", "utf8");
    fs.writeFileSync(path.join(runtimeDir, "helpers", "guest-agent"), "guest-agent\n", "utf8");
    fs.writeFileSync(path.join(runtimeDir, "helpers", "egress-proxy"), "egress-proxy\n", "utf8");
    fs.writeFileSync(path.join(runtimeDir, "helpers", "container-stack.tar.gz"), "container-stack\n", "utf8");
    await withEnv(
      {
        CTX_AVF_LINUX_GUEST_RUNTIME_DIR: runtimeDir,
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
      },
      (mod) => {
        let spawnCalled = false;
        const resolved = mod.__desktopAutomationConfigTestHooks.ensureAutomationAvfLinuxGuestRuntime({
          platform: "darwin",
          runsContainerScenarios: true,
          env: process.env,
          log: (line) => logs.push(line),
          spawnSyncImpl: () => {
            spawnCalled = true;
            return { status: 0 };
          },
        });
        assert.equal(resolved, runtimeDir);
        assert.equal(process.env.CTX_AVF_LINUX_GUEST_RUNTIME_DIR, runtimeDir);
        assert.equal(spawnCalled, false);
      },
    );
    assert.ok(logs.some((line) => line.includes("CTX_AVF_LINUX_GUEST_RUNTIME_DIR")));
  } finally {
    fs.rmSync(runtimeDir, { recursive: true, force: true });
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});

test("wdio automation AVF runtime prep prepares and exports a missing runtime directory", async () => {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-avf-runtime-parent-"));
  const runtimeDir = path.join(tempRoot, "runtime");
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-state-"));
  try {
    await withEnv(
      {
        CTX_AVF_LINUX_GUEST_RUNTIME_DIR: null,
        CTX_AUTOMATION_AVF_LINUX_GUEST_RUNTIME_DIR: runtimeDir,
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
      },
      (mod) => {
        let preparedPath = null;
        const resolved = mod.__desktopAutomationConfigTestHooks.ensureAutomationAvfLinuxGuestRuntime({
          platform: "darwin",
          runsContainerScenarios: true,
          env: process.env,
          spawnSyncImpl: (_cmd, args) => {
            preparedPath = args[2];
            fs.mkdirSync(path.join(preparedPath, "helpers"), { recursive: true });
            fs.writeFileSync(path.join(preparedPath, "rootfs.raw"), "rootfs\n", "utf8");
            fs.writeFileSync(path.join(preparedPath, "helpers", "kernel"), "kernel\n", "utf8");
            fs.writeFileSync(path.join(preparedPath, "helpers", "initrd"), "initrd\n", "utf8");
            fs.writeFileSync(path.join(preparedPath, "helpers", "guest-agent"), "guest-agent\n", "utf8");
            fs.writeFileSync(path.join(preparedPath, "helpers", "egress-proxy"), "egress-proxy\n", "utf8");
            fs.writeFileSync(path.join(preparedPath, "helpers", "container-stack.tar.gz"), "container-stack\n", "utf8");
            return { status: 0 };
          },
          log: () => {},
        });
        assert.equal(preparedPath, runtimeDir);
        assert.equal(resolved, runtimeDir);
        assert.equal(process.env.CTX_AVF_LINUX_GUEST_RUNTIME_DIR, runtimeDir);
      },
    );
  } finally {
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
