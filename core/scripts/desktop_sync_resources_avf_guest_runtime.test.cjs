const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("fs");
const os = require("os");
const path = require("path");

const { stageAvfLinuxGuestRuntime } = require("./desktop_sync_resources.cjs");

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
  fs.writeFileSync(path.join(sourceDir, "version.txt"), "dev-runtime\n", "utf8");

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
