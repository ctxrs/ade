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

const writeAvfRuntimeBundle = (bundleDir, { includeGuestAgent = true } = {}) => {
  const hostOs = desktopOs();
  const hostArch = desktopArch();
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
            },
          },
          {
            kind: "image",
            id: "ctx-harness",
            os: "linux",
            arch: hostArch,
            variant: "default",
            version: "test",
            sources: [{ source_type: "ci", uri: "locked://image", sha256: "5".repeat(64) }],
          },
        ],
      },
      null,
      2,
    ),
    "utf8",
  );
};

const writeThinBundleManifestAndRuntimeLock = (bundleDir, { includeGuestAgent = true } = {}) => {
  const hostOs = desktopOs();
  const hostArch = desktopArch();
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
          {
            kind: "image",
            id: "ctx-harness",
            os: "linux",
            arch: hostArch,
            variant: "default",
            version: "test",
            sources: [{ source_type: "ci", uri: "locked://image", sha256: "5".repeat(64) }],
          },
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
    writeAvfRuntimeBundle(bundleDir);
    await withEnv(
      {
        CTX_BUNDLE_DIR: bundleDir,
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
      },
      (mod) => {
        assert.doesNotThrow(() => {
          mod.__desktopAutomationConfigTestHooks.ensureBundledContainerAssets();
        });
      },
    );
  } finally {
    fs.rmSync(bundleDir, { recursive: true, force: true });
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});

test("wdio AVF container preflight rejects a bundle missing the guest agent helper", async () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-avf-assets-"));
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-state-"));
  try {
    writeAvfRuntimeBundle(bundleDir, { includeGuestAgent: false });
    await withEnv(
      {
        CTX_BUNDLE_DIR: bundleDir,
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
      },
      (mod) => {
        assert.throws(
          () => mod.__desktopAutomationConfigTestHooks.ensureBundledContainerAssets(),
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
    writeThinBundleManifestAndRuntimeLock(bundleDir);
    await withEnv(
      {
        CTX_BUNDLE_DIR: bundleDir,
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
      },
      (mod) => {
        assert.doesNotThrow(() => {
          mod.__desktopAutomationConfigTestHooks.ensureBundledContainerAssets();
        });
      },
    );
  } finally {
    fs.rmSync(bundleDir, { recursive: true, force: true });
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});

test("wdio AVF container preflight rejects a thin bundle missing managed AVF helper metadata", async () => {
  const bundleDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-avf-assets-"));
  const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-wdio-state-"));
  try {
    writeThinBundleManifestAndRuntimeLock(bundleDir, { includeGuestAgent: false });
    await withEnv(
      {
        CTX_BUNDLE_DIR: bundleDir,
        CTX_AUTOMATION_CN_BACKEND_STATE_DIR: stateDir,
      },
      (mod) => {
        assert.throws(
          () => mod.__desktopAutomationConfigTestHooks.ensureBundledContainerAssets(),
          /guest-agent/,
        );
      },
    );
  } finally {
    fs.rmSync(bundleDir, { recursive: true, force: true });
    fs.rmSync(stateDir, { recursive: true, force: true });
  }
});
