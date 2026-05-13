const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  buildLinuxAppDirLaunchEnv,
  resolveLinuxAppDirFromPath,
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

test("linux AppDir launch env resolves extracted AppRun without changing the application path", async () => {
  await withPlatform("linux", async () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-linux-appdir-env-"));
    try {
      const appDir = path.join(tmp, "squashfs-root");
      const appRun = path.join(appDir, "AppRun");
      fs.mkdirSync(path.join(appDir, "usr", "bin"), { recursive: true });
      fs.writeFileSync(appRun, "#!/bin/sh\nexit 0\n", { mode: 0o755 });
      fs.writeFileSync(path.join(appDir, "usr", "bin", "ctx"), "", { mode: 0o755 });

      assert.equal(resolveLinuxAppDirFromPath({ appPath: appRun, env: {} }), appDir);
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
      assert.match(env.PATH, new RegExp(`^${path.join(appDir, "usr", "bin").replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}:`));
      assert.match(env.LD_LIBRARY_PATH, new RegExp(`^${path.join(appDir, "usr", "lib").replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}:`));
      assert.match(env.XDG_DATA_DIRS, new RegExp(`^${path.join(appDir, "usr", "share").replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}:`));
      assert.equal(env.GDK_BACKEND, "x11");
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
