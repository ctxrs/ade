const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("fs");
const os = require("os");
const path = require("path");

const {
  __desktopSyncResourcesTestHooks,
  copySidecarBinary,
  parseAvfLinuxGuestRuntimeVersion,
  stageAvfLinuxGuestRuntime,
} = require("./desktop_sync_resources.cjs");

const hostManifestOs = process.platform === "darwin" ? "macos" : process.platform === "win32" ? "windows" : "linux";
const hostManifestArch = process.arch === "arm64" ? "aarch64" : process.arch === "x64" ? "x86_64" : process.arch;

const withEnv = (vars, fn) => {
  const previous = new Map();
  for (const [key, value] of Object.entries(vars)) {
    previous.set(key, process.env[key]);
    if (value === undefined) {
      delete process.env[key];
    } else {
      process.env[key] = value;
    }
  }
  try {
    return fn();
  } finally {
    for (const [key, value] of previous.entries()) {
      if (value === undefined) {
        delete process.env[key];
      } else {
        process.env[key] = value;
      }
    }
  }
};

test("stageAvfLinuxGuestRuntime copies local guest runtime into bundle manifest", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-avf-guest-runtime-"));
  const sourceDir = path.join(tmpRoot, "source");
  const bundleDir = path.join(tmpRoot, "bundle");

  fs.mkdirSync(path.join(sourceDir, "helpers"), { recursive: true });
  fs.writeFileSync(path.join(sourceDir, "rootfs.raw"), "rootfs\n", "utf8");
  fs.writeFileSync(path.join(sourceDir, "helpers", "kernel"), "kernel\n", "utf8");
  fs.writeFileSync(path.join(sourceDir, "helpers", "initrd"), "initrd\n", "utf8");
  fs.writeFileSync(path.join(sourceDir, "helpers", "guest-agent"), "guest-agent\n", "utf8");
  fs.writeFileSync(path.join(sourceDir, "helpers", "egress-proxy"), "egress-proxy\n", "utf8");
  fs.writeFileSync(path.join(sourceDir, "helpers", "container-stack.tar.gz"), "container-stack\n", "utf8");
  fs.writeFileSync(
    path.join(sourceDir, "version.txt"),
    "version=dev-runtime\nubuntu-release=noble\nubuntu-arch=arm64\n",
    "utf8",
  );

  fs.mkdirSync(bundleDir, { recursive: true });
  fs.writeFileSync(
    path.join(bundleDir, "manifest.json"),
    JSON.stringify({ version: 1, providers: [], runtimes: [], images: [], daemons: [] }, null, 2),
    "utf8",
  );

  const staged = withEnv(
    {
      CTX_AVF_LINUX_GUEST_RUNTIME_DIR: sourceDir,
      CTX_AVF_LINUX_GUEST_RUNTIME_VERSION: "",
    },
    () => stageAvfLinuxGuestRuntime(bundleDir),
  );

  assert.ok(staged, "expected staged runtime metadata");
  assert.equal(staged.version, "dev-runtime");
  assert.ok(fs.existsSync(staged.rootfsPath));
  assert.ok(fs.existsSync(staged.kernelPath));
  assert.ok(fs.existsSync(staged.initrdPath));
  assert.ok(staged.guestAgentPath, "expected optional guest agent to be surfaced");
  assert.ok(fs.existsSync(staged.guestAgentPath));
  assert.ok(staged.egressProxyPath, "expected optional egress proxy to be surfaced");
  assert.ok(fs.existsSync(staged.egressProxyPath));
  assert.ok(staged.containerStackPath, "expected guest container-stack payload to be surfaced");
  assert.ok(fs.existsSync(staged.containerStackPath));

  const manifest = JSON.parse(fs.readFileSync(path.join(bundleDir, "manifest.json"), "utf8"));
  const runtime = manifest.runtimes.find(
    (entry) =>
      entry.id === "avf-linux-guest"
      && entry.os === hostManifestOs
      && entry.arch === hostManifestArch,
  );
  assert.ok(runtime, "expected avf-linux-guest runtime entry in manifest");
  assert.equal(runtime.version, "dev-runtime");
  assert.equal(runtime.bin, "rootfs.raw");
  assert.match(runtime.root, new RegExp(`runtimes[/\\\\]avf-linux-guest[/\\\\]${hostManifestOs}[/\\\\]${hostManifestArch}`));

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});

test("stageAvfLinuxGuestRuntime can use an explicit staged runtime without copying rootfs", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-avf-guest-runtime-external-"));
  const sourceDir = path.join(tmpRoot, "source");
  const bundleDir = path.join(tmpRoot, "bundle");

  fs.mkdirSync(path.join(sourceDir, "helpers"), { recursive: true });
  fs.writeFileSync(path.join(sourceDir, "rootfs.raw"), "rootfs\n", "utf8");
  fs.writeFileSync(path.join(sourceDir, "helpers", "kernel"), "kernel\n", "utf8");
  fs.writeFileSync(path.join(sourceDir, "helpers", "initrd"), "initrd\n", "utf8");
  fs.writeFileSync(path.join(sourceDir, "helpers", "guest-agent"), "guest-agent\n", "utf8");
  fs.writeFileSync(path.join(sourceDir, "helpers", "egress-proxy"), "egress-proxy\n", "utf8");
  fs.writeFileSync(path.join(sourceDir, "helpers", "container-stack.tar.gz"), "container-stack\n", "utf8");
  fs.writeFileSync(path.join(sourceDir, "version.txt"), "version=dev-runtime\n", "utf8");

  fs.mkdirSync(bundleDir, { recursive: true });
  fs.writeFileSync(
    path.join(bundleDir, "manifest.json"),
    JSON.stringify({ version: 1, providers: [], runtimes: [], images: [], daemons: [] }, null, 2),
    "utf8",
  );

  const staged = withEnv(
    {
      CTX_AVF_LINUX_GUEST_RUNTIME_DIR: sourceDir,
      CTX_DESKTOP_STAGE_AVF_LINUX_GUEST_RUNTIME: "0",
    },
    () => stageAvfLinuxGuestRuntime(bundleDir),
  );

  assert.ok(staged, "expected staged runtime metadata");
  assert.equal(staged.copiedIntoBundle, false);
  assert.equal(staged.runtimeRootDir, path.resolve(sourceDir));
  assert.equal(staged.rootfsPath, path.join(path.resolve(sourceDir), "rootfs.raw"));
  assert.equal(
    fs.existsSync(path.join(bundleDir, "runtimes", "avf-linux-guest")),
    false,
    "expected rootfs not to be copied into the desktop bundle",
  );
  const manifest = JSON.parse(fs.readFileSync(path.join(bundleDir, "manifest.json"), "utf8"));
  assert.deepEqual(manifest.runtimes, []);

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});

test("stageAvfLinuxGuestRuntime hashes large staged rootfs without readFileSync", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-avf-guest-runtime-large-"));
  const sourceDir = path.join(tmpRoot, "source");
  const bundleDir = path.join(tmpRoot, "bundle");

  fs.mkdirSync(path.join(sourceDir, "helpers"), { recursive: true });
  fs.writeFileSync(path.join(sourceDir, "rootfs.raw"), "rootfs\n", "utf8");
  fs.writeFileSync(path.join(sourceDir, "helpers", "kernel"), "kernel\n", "utf8");
  fs.writeFileSync(path.join(sourceDir, "helpers", "initrd"), "initrd\n", "utf8");
  fs.writeFileSync(path.join(sourceDir, "helpers", "guest-agent"), "guest-agent\n", "utf8");
  fs.writeFileSync(path.join(sourceDir, "helpers", "egress-proxy"), "egress-proxy\n", "utf8");
  fs.writeFileSync(path.join(sourceDir, "helpers", "container-stack.tar.gz"), "container-stack\n", "utf8");
  fs.writeFileSync(
    path.join(sourceDir, "version.txt"),
    "version=dev-runtime\nubuntu-release=noble\nubuntu-arch=arm64\n",
    "utf8",
  );

  fs.mkdirSync(bundleDir, { recursive: true });
  fs.writeFileSync(
    path.join(bundleDir, "manifest.json"),
    JSON.stringify({ version: 1, providers: [], runtimes: [], images: [], daemons: [] }, null, 2),
    "utf8",
  );

  const originalReadFileSync = fs.readFileSync;
  let blockedRootfsReads = 0;
  fs.readFileSync = (...args) => {
    const [filePath] = args;
    if (typeof filePath === "string" && filePath.endsWith(`${path.sep}rootfs.raw`) && filePath.includes(`${path.sep}bundle${path.sep}`)) {
      blockedRootfsReads += 1;
      throw new Error(`unexpected buffered read of staged rootfs: ${filePath}`);
    }
    return originalReadFileSync.apply(fs, args);
  };

  try {
    const staged = withEnv(
      {
        CTX_AVF_LINUX_GUEST_RUNTIME_DIR: sourceDir,
        CTX_AVF_LINUX_GUEST_RUNTIME_VERSION: "",
      },
      () => stageAvfLinuxGuestRuntime(bundleDir),
    );
    assert.ok(staged);
    assert.equal(blockedRootfsReads, 0);
  } finally {
    fs.readFileSync = originalReadFileSync;
    fs.rmSync(tmpRoot, { recursive: true, force: true });
  }
});

test("parseAvfLinuxGuestRuntimeVersion handles metadata-style version files", () => {
  assert.equal(
    parseAvfLinuxGuestRuntimeVersion("version=ubuntu-noble-arm64-deadbeef\nubuntu-release=noble\n"),
    "ubuntu-noble-arm64-deadbeef",
  );
  assert.equal(
    parseAvfLinuxGuestRuntimeVersion("dev-runtime\nubuntu-release=noble\n"),
    "dev-runtime",
  );
  assert.equal(parseAvfLinuxGuestRuntimeVersion("ubuntu-release=noble\n"), "");
});

test("copySidecarBinary honors an explicit source path override", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-sidecar-"));
  const sourcePath = path.join(tmpRoot, "ctx");
  const destDir = path.join(tmpRoot, "dest");
  fs.writeFileSync(sourcePath, "#!/bin/sh\nexit 0\n", "utf8");

  try {
    const copied = copySidecarBinary({
      sourceDir: path.join(tmpRoot, "missing"),
      sourcePath,
      destDir,
      sourceName: "ctx",
      destName: "ctx-daemon",
      targetTriple: "x86_64-unknown-linux-gnu",
      binExtOverride: "",
      platform: "linux",
    });

    assert.equal(copied.src, sourcePath);
    assert.ok(fs.existsSync(copied.dest));
    assert.ok(fs.existsSync(copied.destTarget));
  } finally {
    fs.rmSync(tmpRoot, { recursive: true, force: true });
  }
});

test("bundle reset preserves tracked lock files and recreates a placeholder manifest", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-bundle-reset-"));
  const bundleDir = path.join(tmpRoot, "bundles");

  fs.mkdirSync(bundleDir, { recursive: true });
  fs.writeFileSync(path.join(bundleDir, "README.md"), "bundle docs\n", "utf8");
  fs.writeFileSync(path.join(bundleDir, "tauri_tools_lock.v1.json"), "{\n  \"schema_version\": 1\n}\n", "utf8");
  fs.writeFileSync(path.join(bundleDir, "runtime_lock.v1.json"), "{\n  \"schema_version\": 1\n}\n", "utf8");
  fs.writeFileSync(path.join(bundleDir, "runtime_lock.v2.json"), "{\n  \"schema_version\": 2\n}\n", "utf8");
  fs.writeFileSync(path.join(bundleDir, "manifest.json"), "{\n  \"version\": 1\n}\n", "utf8");
  fs.writeFileSync(path.join(bundleDir, "scratch.txt"), "delete me\n", "utf8");

  try {
    __desktopSyncResourcesTestHooks.resetBundleDir(bundleDir);
    assert.ok(fs.existsSync(path.join(bundleDir, "tauri_tools_lock.v1.json")));
    assert.ok(fs.existsSync(path.join(bundleDir, "runtime_lock.v1.json")));
    assert.ok(fs.existsSync(path.join(bundleDir, "runtime_lock.v2.json")));
    assert.ok(!fs.existsSync(path.join(bundleDir, "scratch.txt")));

    __desktopSyncResourcesTestHooks.writePlaceholderBundleManifest(bundleDir);
    const manifest = JSON.parse(fs.readFileSync(path.join(bundleDir, "manifest.json"), "utf8"));
    assert.deepEqual(manifest, {
      version: 1,
      providers: [],
      runtimes: [],
      images: [],
      daemons: [],
    });
  } finally {
    fs.rmSync(tmpRoot, { recursive: true, force: true });
  }
});

test("linux append requests do not force ACP bridge when no providers are required", () => {
  const requests = __desktopSyncResourcesTestHooks.resolveLinuxAppendBundleRequests({
    requiredProviderIds: [],
    bundledRuntimeIds: ["node"],
    requiredImageIds: ["ctx-harness"],
    requiredProviderTargets: [],
    requiredRuntimeTargets: [
      { os: "linux", arch: "aarch64" },
      { os: "linux", arch: "x86_64" },
    ],
    requiredImageTargets: [
      { os: "linux", arch: "aarch64" },
      { os: "linux", arch: "x86_64" },
    ],
    bundleHarnessImages: true,
  });

  assert.deepEqual(requests, [
    {
      arch: "aarch64",
      linuxProviders: "__none__",
      includeBridge: "0",
      needsLinuxRuntime: true,
      shouldBundleLinuxImage: true,
    },
    {
      arch: "x86_64",
      linuxProviders: "__none__",
      includeBridge: "0",
      needsLinuxRuntime: true,
      shouldBundleLinuxImage: true,
    },
  ]);
});

test("linux append requests keep ACP bridge enabled when providers are required", () => {
  const requests = __desktopSyncResourcesTestHooks.resolveLinuxAppendBundleRequests({
    requiredProviderIds: ["amp"],
    bundledRuntimeIds: [],
    requiredImageIds: [],
    requiredProviderTargets: [{ os: "linux", arch: "aarch64" }],
    requiredRuntimeTargets: [],
    requiredImageTargets: [],
    bundleHarnessImages: false,
  });

  assert.deepEqual(requests, [
    {
      arch: "aarch64",
      linuxProviders: "amp",
      includeBridge: "1",
      needsLinuxRuntime: false,
      shouldBundleLinuxImage: false,
    },
  ]);
});
