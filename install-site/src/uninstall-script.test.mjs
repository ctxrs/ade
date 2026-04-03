import test from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, chmodSync, existsSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { renderUninstallScript } from "./uninstall-script.js";

const makeTempDir = (prefix) => mkdtempSync(path.join(tmpdir(), prefix));

const writeExecutable = (filePath, contents) => {
  writeFileSync(filePath, contents);
  chmodSync(filePath, 0o755);
};

const installCommonStubs = (stubDir, { os, uid = "1000", aptLogPath, dpkgStatus = "missing" }) => {
  writeExecutable(
    path.join(stubDir, "uname"),
    `#!/bin/sh
set -eu
case "$1" in
  -s) printf '%s\\n' "${os}" ;;
  *) exit 2 ;;
esac
`,
  );

  writeExecutable(
    path.join(stubDir, "id"),
    `#!/bin/sh
set -eu
case "$1" in
  -u) printf '%s\\n' "${uid}" ;;
  *) exit 2 ;;
esac
`,
  );

  writeExecutable(
    path.join(stubDir, "sudo"),
    `#!/bin/sh
set -eu
exec "$@"
`,
  );

  writeExecutable(
    path.join(stubDir, "apt-get"),
    `#!/bin/sh
set -eu
printf '%s\\n' "$*" > "${aptLogPath}"
exit 0
`,
  );

  writeExecutable(
    path.join(stubDir, "dpkg-query"),
    `#!/bin/sh
set -eu
if [ "${dpkgStatus}" = "installed" ]; then
  printf 'install ok installed'
  exit 0
fi
exit 1
`,
  );

  writeExecutable(
    path.join(stubDir, "osascript"),
    `#!/bin/sh
set -eu
exit 0
`,
  );
};

const runUninstaller = ({ os, args = [], env = {}, uid, dpkgStatus }) => {
  const sandboxDir = makeTempDir("ctx-uninstall-script-");
  const stubDir = path.join(sandboxDir, "stubs");
  const homeDir = path.join(sandboxDir, "home");
  const aptLogPath = path.join(sandboxDir, "apt.log");
  const scriptPath = path.join(sandboxDir, "uninstall.sh");

  mkdirSync(stubDir);
  mkdirSync(homeDir);
  installCommonStubs(stubDir, { os, uid, aptLogPath, dpkgStatus });
  writeFileSync(scriptPath, renderUninstallScript());
  chmodSync(scriptPath, 0o755);

  const result = spawnSync("sh", [scriptPath, ...args], {
    encoding: "utf8",
    env: {
      ...process.env,
      HOME: homeDir,
      PATH: `${stubDir}:${process.env.PATH ?? ""}`,
      ...env,
    },
  });

  return {
    ...result,
    sandboxDir,
    homeDir,
    aptLogPath,
    cleanup() {
      rmSync(sandboxDir, { recursive: true, force: true });
    },
  };
};

test("renderUninstallScript emits an interactive uninstall script", () => {
  const script = renderUninstallScript();
  assert.match(script, /^#!\/bin\/sh/m);
  assert.match(script, /confirm_uninstall/);
  assert.match(script, /--yes\|-y/);
  assert.match(script, /ctx uninstall will remove:/);
  assert.match(script, /interactive confirmation requires \/dev\/tty/);
});

test("uninstall script refuses noninteractive execution without --yes", () => {
  const result = runUninstaller({
    os: "Linux",
    env: {
      CTX_UNINSTALL_OS_RELEASE_PATH: path.join(tmpdir(), "ctx-uninstall-missing-os-release"),
    },
  });

  try {
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /interactive confirmation requires \/dev\/tty/);
  } finally {
    result.cleanup();
  }
});

test("macOS uninstall removes app, data, and AVF state with --yes", () => {
  const result = runUninstaller({
    os: "Darwin",
    uid: "502",
    args: ["--yes"],
  });

  const macPaths = {
    systemApp: path.join(result.sandboxDir, "Applications", "ctx.app"),
    userApp: path.join(result.homeDir, "Applications", "ctx.app"),
    dataDir: path.join(result.homeDir, ".ctx"),
    avfPrivateDir: path.join(result.homeDir, ".ctx-avf-host-private"),
    avfSocketDir: path.join(result.sandboxDir, "tmp", "ctxavf-uid-502"),
    appSupportDir: path.join(result.homeDir, "Library", "Application Support", "rs.ctx.desktop"),
    preferencesFile: path.join(result.homeDir, "Library", "Preferences", "rs.ctx.desktop.plist"),
    webkitDir: path.join(result.homeDir, "Library", "WebKit", "rs.ctx.desktop"),
  };

  for (const target of Object.values(macPaths)) {
    mkdirSync(path.dirname(target), { recursive: true });
    if (target.endsWith(".plist")) {
      writeFileSync(target, "plist");
    } else {
      mkdirSync(target, { recursive: true });
    }
  }

  const rerun = spawnSync("sh", [path.join(result.sandboxDir, "uninstall.sh"), "--yes"], {
    encoding: "utf8",
    env: {
      ...process.env,
      HOME: result.homeDir,
      PATH: `${path.join(result.sandboxDir, "stubs")}:${process.env.PATH ?? ""}`,
      CTX_UNINSTALL_MAC_SYSTEM_APP: macPaths.systemApp,
      CTX_UNINSTALL_MAC_USER_APP: macPaths.userApp,
      CTX_DATA_DIR: macPaths.dataDir,
      CTX_UNINSTALL_AVF_PRIVATE_DIR: macPaths.avfPrivateDir,
      CTX_UNINSTALL_AVF_SOCKET_DIR: macPaths.avfSocketDir,
      CTX_UNINSTALL_MAC_APP_SUPPORT_DIR: macPaths.appSupportDir,
      CTX_UNINSTALL_MAC_PREFERENCES_FILE: macPaths.preferencesFile,
      CTX_UNINSTALL_MAC_WEBKIT_DIR: macPaths.webkitDir,
    },
  });

  try {
    assert.equal(rerun.status, 0, rerun.stderr);
    for (const target of Object.values(macPaths)) {
      assert.equal(existsSync(target), false, `expected ${target} to be removed`);
    }
    assert.match(rerun.stderr, /ctx uninstall complete\./);
  } finally {
    result.cleanup();
  }
});

test("Linux uninstall removes local data, AppImage install, and desktop entry with --yes", () => {
  const result = runUninstaller({
    os: "Linux",
    args: ["--yes"],
  });

  const installDir = path.join(result.homeDir, ".local", "share", "ctx");
  const binDir = path.join(result.homeDir, ".local", "bin");
  const launcherPath = path.join(binDir, "ctx-desktop");
  const xdgDataHome = path.join(result.homeDir, ".local", "share");
  const desktopEntryPath = path.join(xdgDataHome, "applications", "ctx.desktop");
  const dataDir = path.join(result.homeDir, ".ctx");
  const osReleasePath = path.join(result.sandboxDir, "os-release");

  mkdirSync(installDir, { recursive: true });
  mkdirSync(binDir, { recursive: true });
  mkdirSync(path.dirname(desktopEntryPath), { recursive: true });
  mkdirSync(dataDir, { recursive: true });
  writeFileSync(path.join(installDir, "ctx.AppImage"), "appimage");
  writeFileSync(launcherPath, "#!/bin/sh\n");
  writeFileSync(desktopEntryPath, "[Desktop Entry]\n");
  writeFileSync(osReleasePath, "ID=fedora\nID_LIKE=rhel\n");

  const rerun = spawnSync("sh", [path.join(result.sandboxDir, "uninstall.sh"), "--yes"], {
    encoding: "utf8",
    env: {
      ...process.env,
      HOME: result.homeDir,
      PATH: `${path.join(result.sandboxDir, "stubs")}:${process.env.PATH ?? ""}`,
      CTX_INSTALL_DIR: installDir,
      CTX_BIN_DIR: binDir,
      CTX_DATA_DIR: dataDir,
      XDG_DATA_HOME: xdgDataHome,
      CTX_UNINSTALL_OS_RELEASE_PATH: osReleasePath,
    },
  });

  try {
    assert.equal(rerun.status, 0, rerun.stderr);
    assert.equal(existsSync(dataDir), false);
    assert.equal(existsSync(installDir), false);
    assert.equal(existsSync(launcherPath), false);
    assert.equal(existsSync(desktopEntryPath), false);
    assert.equal(existsSync(result.aptLogPath), false);
  } finally {
    result.cleanup();
  }
});

test("Linux uninstall purges installed Debian package before removing local state", () => {
  const result = runUninstaller({
    os: "Linux",
    args: ["--yes"],
    dpkgStatus: "installed",
  });

  const installDir = path.join(result.homeDir, ".local", "share", "ctx");
  const binDir = path.join(result.homeDir, ".local", "bin");
  const launcherPath = path.join(binDir, "ctx-desktop");
  const xdgDataHome = path.join(result.homeDir, ".local", "share");
  const desktopEntryPath = path.join(xdgDataHome, "applications", "ctx.desktop");
  const dataDir = path.join(result.homeDir, ".ctx");
  const osReleasePath = path.join(result.sandboxDir, "os-release");

  mkdirSync(installDir, { recursive: true });
  mkdirSync(binDir, { recursive: true });
  mkdirSync(path.dirname(desktopEntryPath), { recursive: true });
  mkdirSync(dataDir, { recursive: true });
  writeFileSync(launcherPath, "#!/bin/sh\n");
  writeFileSync(desktopEntryPath, "[Desktop Entry]\n");
  writeFileSync(osReleasePath, "ID=ubuntu\nID_LIKE=debian\n");

  const rerun = spawnSync("sh", [path.join(result.sandboxDir, "uninstall.sh"), "--yes"], {
    encoding: "utf8",
    env: {
      ...process.env,
      HOME: result.homeDir,
      PATH: `${path.join(result.sandboxDir, "stubs")}:${process.env.PATH ?? ""}`,
      CTX_INSTALL_DIR: installDir,
      CTX_BIN_DIR: binDir,
      CTX_DATA_DIR: dataDir,
      XDG_DATA_HOME: xdgDataHome,
      CTX_UNINSTALL_OS_RELEASE_PATH: osReleasePath,
      CTX_UNINSTALL_DEBIAN_PACKAGE_NAME: "ctx",
    },
  });

  try {
    assert.equal(rerun.status, 0, rerun.stderr);
    assert.equal(readFileSync(result.aptLogPath, "utf8").trim(), "purge -y ctx");
    assert.equal(existsSync(dataDir), false);
    assert.equal(existsSync(installDir), false);
    assert.equal(existsSync(launcherPath), false);
    assert.equal(existsSync(desktopEntryPath), false);
  } finally {
    result.cleanup();
  }
});
