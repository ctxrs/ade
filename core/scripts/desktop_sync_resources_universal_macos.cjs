const fs = require("fs");
const path = require("path");
const childProcess = require("child_process");

const args = process.argv.slice(2);
const profileIdx = args.indexOf("--profile");
const profile = profileIdx !== -1 ? args[profileIdx + 1] : "release";

const coreRoot = path.resolve(__dirname, "..");
const desktopTauriRoot = path.join(coreRoot, "apps", "desktop", "src-tauri");
const destBinDir = path.join(desktopTauriRoot, "bin");
const destWebDistDir = path.join(desktopTauriRoot, "web", "dist");

const targets = ["aarch64-apple-darwin", "x86_64-apple-darwin"];

const copyDirRecursive = (srcDir, destDir) => {
  fs.mkdirSync(destDir, { recursive: true });
  for (const entry of fs.readdirSync(srcDir, { withFileTypes: true })) {
    const srcPath = path.join(srcDir, entry.name);
    const destPath = path.join(destDir, entry.name);
    if (entry.isDirectory()) {
      copyDirRecursive(srcPath, destPath);
      continue;
    }
    if (entry.isSymbolicLink()) {
      const linkTarget = fs.readlinkSync(srcPath);
      fs.symlinkSync(linkTarget, destPath);
      continue;
    }
    fs.copyFileSync(srcPath, destPath);
  }
};

const ensureExecutable = (filePath) => {
  if (!fs.existsSync(filePath)) return;
  try {
    fs.chmodSync(filePath, 0o755);
  } catch (e) {
    console.warn(`warn: failed to chmod +x ${filePath}: ${e?.message ?? e}`);
  }
};

const lipoSidecar = (name) => {
  const [armTarget, intelTarget] = targets;
  const armSrc = path.join(coreRoot, "target", armTarget, profile, name);
  const intelSrc = path.join(coreRoot, "target", intelTarget, profile, name);
  const dest = path.join(destBinDir, name);

  if (!fs.existsSync(armSrc)) {
    throw new Error(`missing sidecar for ${armTarget}: ${armSrc} (did you build --target ${armTarget}?)`);
  }
  if (!fs.existsSync(intelSrc)) {
    throw new Error(`missing sidecar for ${intelTarget}: ${intelSrc} (did you build --target ${intelTarget}?)`);
  }

  fs.mkdirSync(destBinDir, { recursive: true });
  childProcess.execFileSync("lipo", ["-create", armSrc, intelSrc, "-output", dest], {
    stdio: "inherit",
  });
  ensureExecutable(dest);
  return dest;
};

const copyWebDist = () => {
  const srcDist = path.join(coreRoot, "apps", "web", "dist");
  if (!fs.existsSync(srcDist)) {
    throw new Error(`missing web dist: ${srcDist} (did you run pnpm -C apps/web build?)`);
  }

  fs.rmSync(destWebDistDir, { recursive: true, force: true });
  copyDirRecursive(srcDist, destWebDistDir);
  return destWebDistDir;
};

const main = () => {
  if (process.platform !== "darwin") {
    throw new Error("desktop_sync_resources_universal_macos is intended to run on macOS (darwin)");
  }
  if (profile !== "debug" && profile !== "release") {
    throw new Error(`invalid --profile ${profile}; expected debug|release`);
  }
  if (!fs.existsSync(desktopTauriRoot)) {
    throw new Error(`missing desktop tauri project dir: ${desktopTauriRoot}`);
  }

  fs.mkdirSync(destBinDir, { recursive: true });
  for (const entry of fs.readdirSync(destBinDir)) {
    if (entry === ".gitkeep") continue;
    fs.rmSync(path.join(destBinDir, entry), { recursive: true, force: true });
  }

  const copied = {
    context: lipoSidecar("context"),
    contextMcp: lipoSidecar("context-mcp"),
    webDist: copyWebDist(),
  };

  console.log("desktop_sync_resources_universal_macos:", copied);
};

main();

