const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  buildLinuxAppDirLaunchEnv,
  buildLinuxWebDriverHostEnv,
  createLinuxAppDirLaunchWrapper,
  resolveLinuxAppDirExecutablePath,
  resolveLinuxAppDirFromPath,
  resolveLinuxAppDirWebKitExecPath,
} = require("./linux_appdir_launch_env.cjs");

const originalPlatform = process.platform;

const withPlatform = async (platform, fn) => {
  Object.defineProperty(process, "platform", { configurable: true, value: platform });
  try {
    await fn();
  } finally {
    Object.defineProperty(process, "platform", { configurable: true, value: originalPlatform });
  }
};

test("linux AppDir launch env resolves extracted AppRun", async () => {
  await withPlatform("linux", async () => {
      const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-linux-appdir-env-"));
    try {
      const appDir = path.join(tmp, "squashfs-root");
      const appRun = path.join(appDir, "AppRun");
      const partialWebkitExecPath = path.join(appDir, "lib", "x86_64-linux-gnu", "webkit2gtk-4.1");
      const webkitExecPath = path.join(appDir, "usr", "lib", "x86_64-linux-gnu", "webkit2gtk-4.1");
      fs.mkdirSync(path.join(appDir, "usr", "bin"), { recursive: true });
      fs.mkdirSync(partialWebkitExecPath, { recursive: true });
      fs.mkdirSync(webkitExecPath, { recursive: true });
      fs.writeFileSync(appRun, "#!/bin/sh\nexit 0\n", { mode: 0o755 });
      fs.writeFileSync(path.join(appDir, "usr", "bin", "ctx"), "", { mode: 0o755 });
      fs.writeFileSync(path.join(partialWebkitExecPath, "WebKitNetworkProcess"), "", { mode: 0o755 });
      fs.writeFileSync(path.join(webkitExecPath, "WebKitNetworkProcess"), "", { mode: 0o755 });
      fs.writeFileSync(path.join(webkitExecPath, "WebKitWebProcess"), "", { mode: 0o755 });

      assert.equal(resolveLinuxAppDirFromPath({ appPath: appRun, env: {} }), appDir);
      assert.equal(resolveLinuxAppDirWebKitExecPath({ appDir }), webkitExecPath);
      const env = buildLinuxAppDirLaunchEnv({
        appPath: appRun,
        env: {
          APPIMAGE: path.join(tmp, "ctx.AppImage"),
          PATH: "/usr/bin",
          LD_LIBRARY_PATH: "/usr/lib",
          XDG_DATA_DIRS: "/usr/local/share",
        },
      });

      assert.equal(env.APPDIR, appDir);
      assert.equal(env.APPIMAGE, path.join(tmp, "ctx.AppImage"));
      assert.equal(env.ARGV0, path.join(tmp, "ctx.AppImage"));
      assert.equal(env.CTX_APPIMAGE_PATH, path.join(tmp, "ctx.AppImage"));
      assert.equal(env.WEBKIT_EXEC_PATH, webkitExecPath);
      assert.match(env.PATH, new RegExp(`^${path.join(appDir, "usr", "bin").replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}:`));
      assert.match(env.LD_LIBRARY_PATH, new RegExp(`^${path.join(appDir, "usr", "lib").replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}:`));
      assert.match(env.XDG_DATA_DIRS, new RegExp(`^${path.join(appDir, "usr", "share").replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}:`));
      assert.equal(env.GDK_BACKEND, "x11");
    } finally {
      fs.rmSync(tmp, { recursive: true, force: true });
    }
  });
});

test("linux AppDir launch wrapper materializes explicit env before bundled binary", async () => {
  await withPlatform("linux", async () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-linux-appdir-wrapper-"));
    try {
      const appDir = path.join(tmp, "squashfs-root");
      const appRun = path.join(appDir, "AppRun");
      const innerBinary = path.join(appDir, "usr", "bin", "ctx");
      const partialWebkitExecPath = path.join(appDir, "lib", "x86_64-linux-gnu", "webkit2gtk-4.1");
      const webkitExecPath = path.join(appDir, "usr", "lib", "x86_64-linux-gnu", "webkit2gtk-4.1");
      const wrapperDir = path.join(tmp, "launchers");
      const appImage = path.join(tmp, "ctx.AppImage");
      fs.mkdirSync(path.join(appDir, "usr", "bin"), { recursive: true });
      fs.mkdirSync(partialWebkitExecPath, { recursive: true });
      fs.mkdirSync(webkitExecPath, { recursive: true });
      fs.writeFileSync(appRun, "#!/bin/sh\nexit 0\n", { mode: 0o755 });
      fs.writeFileSync(innerBinary, "#!/bin/sh\nexit 0\n", { mode: 0o755 });
      fs.writeFileSync(path.join(partialWebkitExecPath, "WebKitNetworkProcess"), "", { mode: 0o755 });
      fs.writeFileSync(path.join(webkitExecPath, "WebKitNetworkProcess"), "", { mode: 0o755 });
      fs.writeFileSync(path.join(webkitExecPath, "WebKitWebProcess"), "", { mode: 0o755 });

      const wrapperPath = createLinuxAppDirLaunchWrapper({
        appPath: appRun,
        wrapperDir,
        env: {
          APPIMAGE: appImage,
          APPDIR: path.join(tmp, "stale-appdir"),
          ARGV0: appImage,
          CTX_APPIMAGE_PATH: appImage,
          CTX_AUTOMATION_APP_LAUNCH_LOG: path.join(tmp, "app-launch.log"),
          TAURI_WEBVIEW_AUTOMATION: "true",
        },
      });

      assert.notEqual(wrapperPath, appRun);
      assert.equal(path.dirname(wrapperPath), wrapperDir);
      const wrapper = fs.readFileSync(wrapperPath, "utf8");
      assert.match(wrapper, new RegExp(`export APPDIR='${appDir.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}'`));
      assert.doesNotMatch(wrapper, /stale-appdir/);
      assert.match(wrapper, new RegExp(`export APPIMAGE='${appImage.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}'`));
      assert.match(wrapper, new RegExp(`export WEBKIT_EXEC_PATH='${webkitExecPath.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}'`));
      assert.match(wrapper, /export TAURI_WEBVIEW_AUTOMATION='true'/);
      assert.match(wrapper, /launching Linux AppDir desktop app/);
      assert.match(wrapper, /launch target requested=/);
      assert.match(wrapper, /WEBKIT_EXEC_PATH=/);
      assert.match(wrapper, /desktop app exited status=/);
      assert.match(wrapper, new RegExp(`cd '${appDir.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}'`));
      assert.match(wrapper, new RegExp(`'${innerBinary.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}' "\\$@" >> "\\$CTX_AUTOMATION_APP_LAUNCH_LOG" 2>&1`));
      assert.match(wrapper, new RegExp(`exec '${innerBinary.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}' "\\$@"`));
      assert.doesNotMatch(wrapper, new RegExp(`'${appRun.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}' "\\$@"`));
    } finally {
      fs.rmSync(tmp, { recursive: true, force: true });
    }
  });
});

test("linux AppDir automation executable uses bundled binary from extracted AppRun", async () => {
  await withPlatform("linux", async () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-linux-appdir-binary-"));
    try {
      const appDir = path.join(tmp, "squashfs-root");
      const appRun = path.join(appDir, "AppRun");
      const innerBinary = path.join(appDir, "usr", "bin", "ctx");
      fs.mkdirSync(path.dirname(innerBinary), { recursive: true });
      fs.writeFileSync(appRun, "#!/bin/sh\nexit 0\n", { mode: 0o755 });
      fs.writeFileSync(innerBinary, "#!/bin/sh\nexit 0\n", { mode: 0o755 });

      assert.equal(resolveLinuxAppDirExecutablePath({ appPath: appRun, env: {} }), innerBinary);
    } finally {
      fs.rmSync(tmp, { recursive: true, force: true });
    }
  });
});

test("linux AppDir automation executable ignores stale APPDIR for ordinary binaries", async () => {
  await withPlatform("linux", async () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-linux-appdir-stale-"));
    try {
      const debugBinary = path.join(tmp, "debug", "ctx");
      const staleAppDir = path.join(tmp, "stale-appdir");
      fs.mkdirSync(path.dirname(debugBinary), { recursive: true });
      fs.mkdirSync(path.join(staleAppDir, "usr", "bin"), { recursive: true });
      fs.writeFileSync(debugBinary, "#!/bin/sh\nexit 0\n", { mode: 0o755 });
      fs.writeFileSync(path.join(staleAppDir, "usr", "bin", "ctx"), "#!/bin/sh\nexit 0\n", {
        mode: 0o755,
      });

      assert.equal(
        resolveLinuxAppDirExecutablePath({
          appPath: debugBinary,
          env: { APPDIR: staleAppDir },
        }),
        debugBinary,
      );
    } finally {
      fs.rmSync(tmp, { recursive: true, force: true });
    }
  });
});

test("linux AppDir launch env leaves ordinary debug binaries alone", async () => {
  await withPlatform("linux", async () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-linux-debug-env-"));
    try {
      const debugBinary = path.join(tmp, "ctx");
      fs.writeFileSync(debugBinary, "", { mode: 0o755 });

      assert.equal(resolveLinuxAppDirFromPath({ appPath: debugBinary, env: {} }), "");
      assert.deepEqual(buildLinuxAppDirLaunchEnv({ appPath: debugBinary, env: {} }), {});
    } finally {
      fs.rmSync(tmp, { recursive: true, force: true });
    }
  });
});

test("linux WebDriver host env strips AppDir launch-only variables", async () => {
  await withPlatform("linux", async () => {
    const env = buildLinuxWebDriverHostEnv({
      env: {
        APPDIR: "/tmp/appdir",
        APPIMAGE: "/tmp/ctx.AppImage",
        ARGV0: "/tmp/ctx.AppImage",
        CTX_APPIMAGE_PATH: "/tmp/ctx.AppImage",
        CTX_AUTOMATION_APP_LAUNCH_LOG: "/tmp/app-launch.log",
        CTX_BUNDLE_DIR: "/tmp/appdir/usr/lib/ctx/bundles",
        CTX_DESKTOP_DAEMON_DATA_DIR: "/tmp/daemon",
        DISPLAY: ":99",
        HOME: "/tmp/home",
        LD_LIBRARY_PATH: "/tmp/appdir/usr/lib:/usr/lib",
        PATH: "/usr/bin:/bin",
        TAURI_WEBVIEW_AUTOMATION: "true",
        WEBKIT_EXEC_PATH: "/tmp/appdir/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1",
        XDG_RUNTIME_DIR: "/tmp/runtime",
      },
    });

    assert.equal(env.APPDIR, undefined);
    assert.equal(env.APPIMAGE, undefined);
    assert.equal(env.CTX_AUTOMATION_APP_LAUNCH_LOG, undefined);
    assert.equal(env.CTX_DESKTOP_DAEMON_DATA_DIR, undefined);
    assert.equal(env.LD_LIBRARY_PATH, undefined);
    assert.equal(env.TAURI_WEBVIEW_AUTOMATION, undefined);
    assert.equal(env.WEBKIT_EXEC_PATH, undefined);
    assert.equal(env.DISPLAY, ":99");
    assert.equal(env.HOME, "/tmp/home");
    assert.equal(env.PATH, "/usr/bin:/bin");
    assert.equal(env.XDG_RUNTIME_DIR, "/tmp/runtime");
  });
});
