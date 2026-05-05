const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  resolveRemoteDaemonBundleTargetArches,
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

test("remote daemon bundle target arches default to release-complete coverage", () => {
  assert.deepEqual(resolveRemoteDaemonBundleTargetArches({}), ["x86_64", "aarch64"]);
});

test("remote daemon bundle target arches accept remote workspace arch aliases", () => {
  assert.deepEqual(
    resolveRemoteDaemonBundleTargetArches({
      CTX_BUNDLE_REMOTE_DAEMON_ARCHES: "linux-x64,linux-arm64",
    }),
    ["x86_64", "aarch64"],
  );
});

test("remote daemon bundle target arches can scope source-app acceptance to x64", () => {
  assert.deepEqual(
    resolveRemoteDaemonBundleTargetArches({ CTX_BUNDLE_REMOTE_DAEMON_ARCHES: "linux-x64" }),
    ["x86_64"],
  );
});

test("remote daemon bundle target arches reject unsupported values", () => {
  assert.throws(
    () => resolveRemoteDaemonBundleTargetArches({ CTX_BUNDLE_REMOTE_DAEMON_ARCHES: "sparc" }),
    /Invalid CTX_BUNDLE_REMOTE_DAEMON_ARCHES entry/,
  );
});

test("remote daemon container build command creates /out before install", () => {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "desktop-sync-test-"));
  const daemonsDir = path.join(tempRoot, "daemons");
  const [spawnCmd, args] = __desktopSyncResourcesTestHooks.buildRemoteDaemonContainerArgs({
    runtime: "docker",
    builderImage: "rust:test",
    coreDir: "/src-host",
    daemonsDir,
    targetCache: "/target-host",
    cargoHome: "/cargo-home-host",
    rustupHome: "/rustup-home-host",
    target: {
      platform: "linux/amd64",
      rustTarget: "x86_64-unknown-linux-gnu",
      fileName: "ctx-daemon-linux-x86_64",
    },
    identity: {
      exactVersion: "0.62.22-preview.deadbeef",
      buildId: "deadbeef",
      compatibilityToken: "artifact-deadbeef",
    },
    hostOs: "linux",
  });

  assert.equal(spawnCmd, "docker");
  assert.match(args.join(" "), /mkdir -p \/out;/);
  assert.match(args.join(" "), new RegExp(`--user ${process.getuid()}:${process.getgid()}`));
  assert.match(args.join(" "), /rustup toolchain install stable --profile minimal --no-self-update/);
  assert.match(args.join(" "), /rustup target add --toolchain stable x86_64-unknown-linux-gnu/);
  assert.match(args.join(" "), /cargo \+stable build --manifest-path \/src\/Cargo\.toml -p ctx-http --release --target x86_64-unknown-linux-gnu/);
  assert.match(args.join(" "), /-v \/cargo-home-host:\/cargo-home/);
  assert.match(args.join(" "), /-v \/rustup-home-host:\/rustup-home/);
  assert.match(args.join(" "), /-e CARGO_HOME=\/cargo-home/);
  assert.match(args.join(" "), /-e RUSTUP_HOME=\/rustup-home/);
  assert.match(args.join(" "), /-e CTX_RELEASE_EFFECTIVE_VERSION=0\.62\.22-preview\.deadbeef/);
  assert.match(args.join(" "), /-e CTX_BUILD_ID=deadbeef/);
  assert.match(args.join(" "), /-e CTX_COMPATIBILITY_TOKEN=artifact-deadbeef/);
  assert.match(args.join(" "), /install -Dm0755 .* \/out\/ctx-daemon-linux-x86_64/);
  assert.doesNotMatch(args.join(" "), /\/usr\/local\/cargo\/registry/);
  assert.doesNotMatch(args.join(" "), /\/usr\/local\/cargo\/git/);

  fs.rmSync(tempRoot, { recursive: true, force: true });
});

test("linux ctx-mcp container build command stages runtime into bundle tree", () => {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "desktop-sync-test-"));
  const runtimesDir = path.join(tempRoot, "bundle");
  const [spawnCmd, args] = __desktopSyncResourcesTestHooks.buildLinuxCtxMcpContainerArgs({
    runtime: "docker",
    builderImage: "rust:test",
    coreDir: "/src-host",
    runtimesDir,
    targetCache: "/target-host",
    cargoHome: "/cargo-home-host",
    rustupHome: "/rustup-home-host",
    target: {
      arch: "aarch64",
      platform: "linux/arm64",
      rustTarget: "aarch64-unknown-linux-gnu",
    },
    runtimeVersion: "0.1.0",
    identity: {
      exactVersion: "0.62.22-preview.deadbeef",
      buildId: "deadbeef",
      compatibilityToken: "artifact-deadbeef",
    },
    hostOs: "linux",
  });

  assert.equal(spawnCmd, "docker");
  assert.match(args.join(" "), /mkdir -p \/out;/);
  assert.match(args.join(" "), new RegExp(`--user ${process.getuid()}:${process.getgid()}`));
  assert.match(args.join(" "), /-p ctx-mcp/);
  assert.match(args.join(" "), /rustup toolchain install stable --profile minimal --no-self-update/);
  assert.match(args.join(" "), /rustup target add --toolchain stable aarch64-unknown-linux-gnu/);
  assert.match(args.join(" "), /cargo \+stable build --manifest-path \/src\/Cargo\.toml -p ctx-mcp --release --target aarch64-unknown-linux-gnu/);
  assert.match(args.join(" "), /-v \/cargo-home-host:\/cargo-home/);
  assert.match(args.join(" "), /-v \/rustup-home-host:\/rustup-home/);
  assert.match(args.join(" "), /-e CARGO_HOME=\/cargo-home/);
  assert.match(args.join(" "), /-e RUSTUP_HOME=\/rustup-home/);
  assert.match(args.join(" "), /-e CTX_RELEASE_EFFECTIVE_VERSION=0\.62\.22-preview\.deadbeef/);
  assert.match(args.join(" "), /-e CTX_BUILD_ID=deadbeef/);
  assert.match(args.join(" "), /-e CTX_COMPATIBILITY_TOKEN=artifact-deadbeef/);
  assert.match(
    args.join(" "),
    /install -Dm0755 .* \/out\/runtimes\/ctx-mcp\/linux\/aarch64\/0\.1\.0\/ctx-mcp/,
  );
  assert.match(args.join(" "), /chmod -R 0777 \/out\/runtimes\/ctx-mcp/);
  assert.doesNotMatch(args.join(" "), /\/usr\/local\/cargo\/registry/);
  assert.doesNotMatch(args.join(" "), /\/usr\/local\/cargo\/git/);

  fs.rmSync(tempRoot, { recursive: true, force: true });
});

test("bundle cache root follows CARGO_TARGET_DIR before HOME cache fallbacks", () => {
  const cacheRoot = __desktopSyncResourcesTestHooks.resolveBundleCacheRoot(
    "desktop-remote-daemons",
    {
      CARGO_TARGET_DIR: "/tmp/ctx-cargo-target",
      HOME: "/Users/example-user",
    },
  );

  assert.equal(
    cacheRoot,
    "/tmp/ctx-cargo-target/desktop-sync-cache/desktop-remote-daemons",
  );
});

test("bundle script env preserves cargo bin on PATH when CARGO_HOME is set", () => {
  const env = __desktopSyncResourcesTestHooks.ensureCargoBinOnPath({
    CARGO_HOME: "/tmp/cargo-home",
    PATH: "/usr/bin:/bin",
  });

  assert.equal(
    env.PATH,
    `${path.join("/tmp/cargo-home", "bin")}${path.delimiter}/usr/bin:/bin`,
  );
});

test("bundle script env does not duplicate cargo bin on PATH", () => {
  const cargoBin = path.join("/tmp/cargo-home", "bin");
  const env = __desktopSyncResourcesTestHooks.ensureCargoBinOnPath({
    CARGO_HOME: "/tmp/cargo-home",
    PATH: `${cargoBin}${path.delimiter}/usr/bin:/bin`,
  });

  assert.equal(env.PATH, `${cargoBin}${path.delimiter}/usr/bin:/bin`);
});

test("container cache dirs on macOS are made world-writable for docker bind mounts", () => {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "desktop-sync-test-"));
  const cacheDir = path.join(tempRoot, "cargo-home");
  fs.rmSync(tempRoot, { recursive: true, force: true });

  __desktopSyncResourcesTestHooks.ensureContainerCacheDir(cacheDir, "macos");

  const mode = fs.statSync(cacheDir).mode & 0o777;
  assert.equal(mode, 0o777);

  fs.rmSync(tempRoot, { recursive: true, force: true });
});

test("linux container builds run as the host uid and gid", () => {
  assert.deepEqual(
    __desktopSyncResourcesTestHooks.containerHostUserArgs("linux"),
    ["--user", `${process.getuid()}:${process.getgid()}`],
  );
});

test("linux container bundle cache preparation normalizes bind mount ownership", () => {
  const [spawnCmd, args] = __desktopSyncResourcesTestHooks.buildContainerWritableMountPrepareArgs({
    runtime: "docker",
    builderImage: "rust:test",
    dirs: ["/target-host", "/cargo-home-host", "/target-host"],
    hostOs: "linux",
    uid: 1234,
    gid: 5678,
  });

  assert.equal(spawnCmd, "docker");
  assert.deepEqual(args.slice(0, 2), ["run", "--rm"]);
  assert.doesNotMatch(args.join(" "), /--user /);
  assert.match(args.join(" "), /-v \/target-host:\/mnt\/ctx-cache-0/);
  assert.match(args.join(" "), /-v \/cargo-home-host:\/mnt\/ctx-cache-1/);
  assert.doesNotMatch(args.join(" "), /ctx-cache-2/);
  assert.match(args.join(" "), /chown -R 1234:5678 "\$dir"/);
  assert.match(args.join(" "), /chmod -R u\+rwX,g\+rwX "\$dir"/);
});

test("macOS container bundle cache preparation is not needed", () => {
  assert.equal(
    __desktopSyncResourcesTestHooks.buildContainerWritableMountPrepareArgs({
      runtime: "docker",
      builderImage: "rust:test",
      dirs: ["/target-host"],
      hostOs: "macos",
    }),
    null,
  );
});

test("macOS container builds keep default docker user mapping", () => {
  assert.deepEqual(
    __desktopSyncResourcesTestHooks.containerHostUserArgs("macos"),
    [],
  );
});

test("remote daemon output mount dirs on macOS are made world-writable for docker bind mounts", () => {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "desktop-sync-test-"));
  const daemonsDir = path.join(tempRoot, "daemons");
  fs.mkdirSync(daemonsDir, { recursive: true });
  fs.chmodSync(daemonsDir, 0o755);

  const [, args] = __desktopSyncResourcesTestHooks.buildRemoteDaemonContainerArgs({
    runtime: "docker",
    builderImage: "rust:test",
    coreDir: "/src-host",
    daemonsDir,
    targetCache: "/target-host",
    cargoHome: "/cargo-home-host",
    rustupHome: "/rustup-home-host",
    target: {
      platform: "linux/amd64",
      rustTarget: "x86_64-unknown-linux-gnu",
      fileName: "ctx-daemon-linux-x86_64",
    },
    hostOs: "macos",
  });

  const mode = fs.statSync(daemonsDir).mode & 0o777;
  assert.equal(mode, 0o777);
  assert.doesNotMatch(args.join(" "), /--user /);

  fs.rmSync(tempRoot, { recursive: true, force: true });
});

test("linux ctx-mcp output mount dirs on macOS are made world-writable for docker bind mounts", () => {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "desktop-sync-test-"));
  const runtimesDir = path.join(tempRoot, "runtimes");
  fs.mkdirSync(runtimesDir, { recursive: true });
  fs.chmodSync(runtimesDir, 0o755);
  const runtimeBundleDir = path.join(runtimesDir, "runtimes");
  fs.mkdirSync(runtimeBundleDir, { recursive: true });
  fs.chmodSync(runtimeBundleDir, 0o755);

  const [, args] = __desktopSyncResourcesTestHooks.buildLinuxCtxMcpContainerArgs({
    runtime: "docker",
    builderImage: "rust:test",
    coreDir: "/src-host",
    runtimesDir,
    targetCache: "/target-host",
    cargoHome: "/cargo-home-host",
    rustupHome: "/rustup-home-host",
    target: {
      arch: "aarch64",
      platform: "linux/arm64",
      rustTarget: "aarch64-unknown-linux-gnu",
    },
    runtimeVersion: "0.1.0",
    hostOs: "macos",
  });

  const rootMode = fs.statSync(runtimesDir).mode & 0o777;
  assert.equal(rootMode, 0o777);
  const nestedMode = fs.statSync(runtimeBundleDir).mode & 0o777;
  assert.equal(nestedMode, 0o777);
  assert.doesNotMatch(args.join(" "), /--user /);

  fs.rmSync(tempRoot, { recursive: true, force: true });
});
