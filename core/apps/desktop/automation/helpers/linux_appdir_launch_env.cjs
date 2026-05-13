const fs = require("fs");
const path = require("path");

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

const prependPath = (entries, current, delimiter = path.delimiter) => {
  const normalizedEntries = entries.map((entry) => String(entry || "").trim()).filter(Boolean);
  const normalizedCurrent = String(current || "").trim();
  return [...normalizedEntries, normalizedCurrent].filter(Boolean).join(delimiter);
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

module.exports = {
  buildLinuxAppDirLaunchEnv,
  resolveLinuxAppDirFromPath,
};
