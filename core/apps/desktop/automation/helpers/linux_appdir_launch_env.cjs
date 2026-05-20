const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const resolveConfiguredPath = (rawValue, pathImpl = path) => {
  const configured = String(rawValue || "").trim();
  if (!configured) return "";
  return pathImpl.isAbsolute(configured) ? configured : pathImpl.resolve(configured);
};

const existingDirectory = (dir, fsImpl = fs) => {
  if (!dir) return "";
  try {
    return fsImpl.existsSync(dir) && fsImpl.statSync(dir).isDirectory() ? dir : "";
  } catch {
    return "";
  }
};

const existingExecutableFile = (filePath, fsImpl = fs) => {
  if (!filePath) return "";
  try {
    const stat = fsImpl.statSync(filePath);
    return stat.isFile() && (stat.mode & 0o111) !== 0 ? filePath : "";
  } catch {
    return "";
  }
};

const resolveLinuxAppDirFromPath = ({
  appPath,
  env = process.env,
  fsImpl = fs,
  pathImpl = path,
} = {}) => {
  if (process.platform !== "linux") return "";
  const normalized = resolveConfiguredPath(appPath, pathImpl);
  const candidates = [];
  if (normalized) {
    if (pathImpl.basename(normalized) === "AppRun") {
      candidates.push(pathImpl.dirname(normalized));
    }
    const binaryMarker = `${pathImpl.sep}usr${pathImpl.sep}bin${pathImpl.sep}ctx`;
    const binaryIndex = normalized.indexOf(binaryMarker);
    if (binaryIndex > 0) {
      candidates.push(normalized.slice(0, binaryIndex));
    }
  }
  const configuredAppDir = resolveConfiguredPath(env.APPDIR, pathImpl);
  if (configuredAppDir) {
    candidates.push(configuredAppDir);
  }
  for (const candidate of candidates) {
    const appDir = existingDirectory(candidate, fsImpl);
    if (appDir) return appDir;
  }
  return "";
};

const resolveLinuxAppDirFromExplicitPath = ({
  appPath,
  fsImpl = fs,
  pathImpl = path,
} = {}) => {
  if (process.platform !== "linux") return "";
  const normalized = resolveConfiguredPath(appPath, pathImpl);
  if (!normalized) return "";
  const candidates = [];
  if (pathImpl.basename(normalized) === "AppRun") {
    candidates.push(pathImpl.dirname(normalized));
  }
  const binaryMarker = `${pathImpl.sep}usr${pathImpl.sep}bin${pathImpl.sep}ctx`;
  const binaryIndex = normalized.indexOf(binaryMarker);
  if (binaryIndex > 0) {
    candidates.push(normalized.slice(0, binaryIndex));
  }
  for (const candidate of candidates) {
    const appDir = existingDirectory(candidate, fsImpl);
    if (appDir) return appDir;
  }
  return "";
};

const resolveLinuxAppDirExecutablePath = ({
  appPath,
  fsImpl = fs,
  pathImpl = path,
} = {}) => {
  if (process.platform !== "linux") return resolveConfiguredPath(appPath, pathImpl);
  const appExecutablePath = resolveConfiguredPath(appPath, pathImpl);
  const appDir = resolveLinuxAppDirFromExplicitPath({ appPath: appExecutablePath, fsImpl, pathImpl });
  if (!appDir) return appExecutablePath;
  return existingExecutableFile(pathImpl.join(appDir, "usr", "bin", "ctx"), fsImpl)
    || appExecutablePath;
};

const prependPath = (entries, current, delimiter = path.delimiter) => {
  const normalizedEntries = entries.map((entry) => String(entry || "").trim()).filter(Boolean);
  const normalizedCurrent = String(current || "").trim();
  return [...normalizedEntries, normalizedCurrent].filter(Boolean).join(delimiter);
};

const resolveLinuxAppDirWebKitExecPath = ({
  appDir,
  fsImpl = fs,
  pathImpl = path,
} = {}) => {
  if (!appDir) return "";
  for (const candidate of [
    pathImpl.join(appDir, "lib", "x86_64-linux-gnu", "webkit2gtk-4.1"),
    pathImpl.join(appDir, "usr", "lib", "x86_64-linux-gnu", "webkit2gtk-4.1"),
    pathImpl.join(appDir, "usr", "libexec", "webkit2gtk-4.1"),
  ]) {
    if (
      existingExecutableFile(pathImpl.join(candidate, "WebKitNetworkProcess"), fsImpl)
      && existingExecutableFile(pathImpl.join(candidate, "WebKitWebProcess"), fsImpl)
    ) {
      return candidate;
    }
  }
  return "";
};

const resolveLinuxAppDirLaunchCwd = ({
  appDir,
  webKitExecPath = "",
  pathImpl = path,
} = {}) => {
  if (!appDir) return "";
  const usrDir = pathImpl.join(appDir, "usr");
  const normalizedWebKitExecPath = resolveConfiguredPath(webKitExecPath, pathImpl);
  if (
    normalizedWebKitExecPath
    && (
      normalizedWebKitExecPath === pathImpl.join(usrDir, "lib", "x86_64-linux-gnu", "webkit2gtk-4.1")
      || normalizedWebKitExecPath.startsWith(`${usrDir}${pathImpl.sep}`)
    )
  ) {
    return usrDir;
  }
  return appDir;
};

const shellQuote = (value) => `'${String(value).replace(/'/g, `'\"'\"'`)}'`;

const LINUX_APPDIR_DRIVER_ENV_STRIP_KEYS = [
  "APPDIR",
  "APPIMAGE",
  "APPIMAGE_EXTRACT_AND_RUN",
  "ARGV0",
  "CTX_APPIMAGE_PATH",
  "CTX_AUTOMATION_APP_LAUNCH_LOG",
  "CTX_BUNDLE_DIR",
  "CTX_DESKTOP_DAEMON_DATA_DIR",
  "GDK_BACKEND",
  "GDK_PIXBUF_MODULE_FILE",
  "GIO_EXTRA_MODULES",
  "GSETTINGS_SCHEMA_DIR",
  "GTK_DATA_PREFIX",
  "GTK_EXE_PREFIX",
  "GTK_IM_MODULE_FILE",
  "GTK_PATH",
  "GTK_THEME",
  "LD_LIBRARY_PATH",
  "TAURI_WEBVIEW_AUTOMATION",
  "WEBKIT_EXEC_PATH",
  "XDG_DATA_DIRS",
];

const buildLinuxWebDriverHostEnv = ({ env = process.env } = {}) => {
  const cleanEnv = { ...env };
  if (process.platform !== "linux") return cleanEnv;
  for (const key of LINUX_APPDIR_DRIVER_ENV_STRIP_KEYS) {
    delete cleanEnv[key];
  }
  return cleanEnv;
};

const buildLinuxAppDirLaunchEnv = ({
  appPath,
  env = process.env,
  fsImpl = fs,
  pathImpl = path,
} = {}) => {
  if (process.platform !== "linux") return {};
  const appDir = resolveLinuxAppDirFromPath({ appPath, env, fsImpl, pathImpl });
  if (!appDir) return {};

  const usrDir = pathImpl.join(appDir, "usr");
  const webKitExecPath = resolveLinuxAppDirWebKitExecPath({ appDir, fsImpl, pathImpl });
  const libDirs = [
    pathImpl.join(usrDir, "lib"),
    pathImpl.join(usrDir, "lib", "x86_64-linux-gnu"),
    pathImpl.join(usrDir, "lib64"),
    pathImpl.join(appDir, "lib"),
    pathImpl.join(appDir, "lib", "x86_64-linux-gnu"),
    pathImpl.join(appDir, "lib64"),
  ];
  const patch = {
    APPDIR: appDir,
    PATH: prependPath(
      [
        pathImpl.join(usrDir, "bin"),
        pathImpl.join(usrDir, "sbin"),
        pathImpl.join(appDir, "bin"),
        pathImpl.join(appDir, "sbin"),
      ],
      env.PATH,
      pathImpl.delimiter,
    ),
    LD_LIBRARY_PATH: prependPath(libDirs, env.LD_LIBRARY_PATH, pathImpl.delimiter),
    XDG_DATA_DIRS: prependPath(
      [pathImpl.join(usrDir, "share"), "/usr/share"],
      env.XDG_DATA_DIRS,
      pathImpl.delimiter,
    ),
    GSETTINGS_SCHEMA_DIR: pathImpl.join(usrDir, "share", "glib-2.0", "schemas"),
    GTK_DATA_PREFIX: appDir,
    GTK_EXE_PREFIX: usrDir,
    GTK_PATH: prependPath(
      [
        pathImpl.join(usrDir, "lib", "x86_64-linux-gnu", "gtk-3.0"),
        "/usr/lib64/gtk-3.0",
        "/usr/lib/x86_64-linux-gnu/gtk-3.0",
      ],
      env.GTK_PATH,
      pathImpl.delimiter,
    ),
    GTK_IM_MODULE_FILE: pathImpl.join(
      usrDir,
      "lib",
      "x86_64-linux-gnu",
      "gtk-3.0",
      "3.0.0",
      "immodules.cache",
    ),
    GDK_PIXBUF_MODULE_FILE: pathImpl.join(
      usrDir,
      "lib",
      "x86_64-linux-gnu",
      "gdk-pixbuf-2.0",
      "2.10.0",
      "loaders.cache",
    ),
    GIO_EXTRA_MODULES: prependPath(
      [pathImpl.join(usrDir, "lib", "x86_64-linux-gnu", "gio", "modules")],
      env.GIO_EXTRA_MODULES,
      pathImpl.delimiter,
    ),
    GDK_BACKEND: String(env.GDK_BACKEND || "").trim() || "x11",
  };
  if (webKitExecPath) {
    patch.WEBKIT_EXEC_PATH = webKitExecPath;
  }

  for (const key of ["APPIMAGE", "APPIMAGE_EXTRACT_AND_RUN", "ARGV0", "CTX_APPIMAGE_PATH"]) {
    const value = String(env[key] || "").trim();
    if (value) {
      patch[key] = value;
    }
  }
  if (!patch.ARGV0 && patch.APPIMAGE) {
    patch.ARGV0 = patch.APPIMAGE;
  }
  if (!patch.CTX_APPIMAGE_PATH && patch.APPIMAGE) {
    patch.CTX_APPIMAGE_PATH = patch.APPIMAGE;
  }
  const gtkTheme = String(env.GTK_THEME || env.APPIMAGE_GTK_THEME || "").trim();
  if (gtkTheme) {
    patch.GTK_THEME = gtkTheme;
  }
  return patch;
};

const createLinuxAppDirLaunchWrapper = ({
  appPath,
  env = {},
  wrapperDir,
  fsImpl = fs,
  pathImpl = path,
  cryptoImpl = crypto,
} = {}) => {
  if (process.platform !== "linux") return appPath;
  const appExecutablePath = resolveConfiguredPath(appPath, pathImpl);
  const appDir = resolveLinuxAppDirFromPath({ appPath: appExecutablePath, env, fsImpl, pathImpl });
  if (!appExecutablePath || !appDir) return appPath;
  const appLaunchTargetPath =
    resolveLinuxAppDirExecutablePath({ appPath: appExecutablePath, fsImpl, pathImpl })
    || appExecutablePath;
  const launchEnvPatch = buildLinuxAppDirLaunchEnv({ appPath: appExecutablePath, env, fsImpl, pathImpl });
  const launchEnv = {
    ...env,
    ...launchEnvPatch,
  };
  const appLaunchCwd = resolveLinuxAppDirLaunchCwd({
    appDir,
    webKitExecPath: launchEnvPatch.WEBKIT_EXEC_PATH,
    pathImpl,
  });
  const entries = Object.entries(launchEnv).filter(([, value]) => String(value || "").trim());
  const normalizedWrapperDir = resolveConfiguredPath(wrapperDir, pathImpl);
  if (!normalizedWrapperDir) {
    throw new Error("Linux AppDir launch wrapper requires wrapperDir.");
  }
  const signature = cryptoImpl
    .createHash("sha256")
    .update(JSON.stringify({ appExecutablePath, appLaunchTargetPath, appLaunchCwd, launchEnv }))
    .digest("hex")
    .slice(0, 16);
  fsImpl.mkdirSync(normalizedWrapperDir, { recursive: true });
  const wrapperPath = pathImpl.join(normalizedWrapperDir, `ctx-linux-appdir-${signature}.sh`);
  const lines = [
    "#!/bin/sh",
    "set -eu",
    ...entries.map(([key, value]) => `export ${key}=${shellQuote(value)}`),
    `cd ${shellQuote(appLaunchCwd)}`,
    "if [ -n \"${CTX_AUTOMATION_APP_LAUNCH_LOG:-}\" ]; then",
    "  mkdir -p \"$(dirname \"$CTX_AUTOMATION_APP_LAUNCH_LOG\")\"",
    "  printf '%s\\n' \"launching Linux AppDir desktop app\" >> \"$CTX_AUTOMATION_APP_LAUNCH_LOG\"",
    `  printf 'launch target requested=%s executable=%s cwd=%s\\n' ${shellQuote(appExecutablePath)} ${shellQuote(appLaunchTargetPath)} ${shellQuote(appLaunchCwd)} >> "$CTX_AUTOMATION_APP_LAUNCH_LOG"`,
    "  printf 'launch args:' >> \"$CTX_AUTOMATION_APP_LAUNCH_LOG\"",
    "  for arg in \"$@\"; do",
    "    printf ' %s' \"$arg\" >> \"$CTX_AUTOMATION_APP_LAUNCH_LOG\"",
    "  done",
    "  printf '\\n' >> \"$CTX_AUTOMATION_APP_LAUNCH_LOG\"",
    "  printf 'launch env TAURI_WEBVIEW_AUTOMATION=%s APPDIR=%s APPIMAGE=%s ARGV0=%s CTX_APPIMAGE_PATH=%s WEBKIT_EXEC_PATH=%s DISPLAY=%s XDG_RUNTIME_DIR=%s HOME=%s\\n' \"${TAURI_WEBVIEW_AUTOMATION:-}\" \"${APPDIR:-}\" \"${APPIMAGE:-}\" \"${ARGV0:-}\" \"${CTX_APPIMAGE_PATH:-}\" \"${WEBKIT_EXEC_PATH:-}\" \"${DISPLAY:-}\" \"${XDG_RUNTIME_DIR:-}\" \"${HOME:-}\" >> \"$CTX_AUTOMATION_APP_LAUNCH_LOG\"",
    "  set +e",
    `  ${shellQuote(appLaunchTargetPath)} "$@" >> "$CTX_AUTOMATION_APP_LAUNCH_LOG" 2>&1`,
    "  status=$?",
    "  set -e",
    "  printf 'desktop app exited status=%s\\n' \"$status\" >> \"$CTX_AUTOMATION_APP_LAUNCH_LOG\"",
    "  exit \"$status\"",
    "fi",
    `exec ${shellQuote(appLaunchTargetPath)} "$@"`,
    "",
  ];
  fsImpl.writeFileSync(wrapperPath, lines.join("\n"), { mode: 0o700 });
  fsImpl.chmodSync(wrapperPath, 0o700);
  return wrapperPath;
};

module.exports = {
  buildLinuxAppDirLaunchEnv,
  buildLinuxWebDriverHostEnv,
  createLinuxAppDirLaunchWrapper,
  resolveLinuxAppDirExecutablePath,
  resolveLinuxAppDirFromPath,
  resolveLinuxAppDirLaunchCwd,
  resolveLinuxAppDirWebKitExecPath,
};
