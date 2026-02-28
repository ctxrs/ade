#!/usr/bin/env node
const fs = require("fs");
const path = require("path");
const { normalizePermissionsRecursive } = require("./desktop_normalize_bundle_permissions.cjs");

const usage = () => {
  console.error("usage: node core/scripts/desktop_import_bundles.cjs --from <bundle-dir>");
};

const parseArgs = (argv) => {
  let source = "";
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    const value = argv[i + 1];
    if (flag === "--from") {
      if (!value) throw new Error("missing value for --from");
      source = value;
      i += 1;
      continue;
    }
    throw new Error(`unknown argument: ${flag}`);
  }
  if (!source) throw new Error("missing required --from");
  return { source };
};

const copyRecursive = (srcDir, destDir) => {
  fs.mkdirSync(destDir, { recursive: true });
  for (const entry of fs.readdirSync(srcDir, { withFileTypes: true })) {
    const srcPath = path.join(srcDir, entry.name);
    const destPath = path.join(destDir, entry.name);
    if (entry.isDirectory()) {
      copyRecursive(srcPath, destPath);
      continue;
    }
    if (entry.isFile()) {
      fs.mkdirSync(path.dirname(destPath), { recursive: true });
      fs.copyFileSync(srcPath, destPath);
    }
  }
};

const main = () => {
  let options;
  try {
    options = parseArgs(process.argv.slice(2));
  } catch (err) {
    usage();
    throw err;
  }

  const sourceDir = path.resolve(options.source);
  const sourceManifest = path.join(sourceDir, "manifest.json");
  if (!fs.existsSync(sourceManifest)) {
    throw new Error(`bundle source missing manifest: ${sourceManifest}`);
  }

  const coreRoot = path.resolve(__dirname, "..");
  const destDir = process.env.CTX_DESKTOP_IMPORT_DEST
    ? path.resolve(process.env.CTX_DESKTOP_IMPORT_DEST)
    : path.join(coreRoot, "apps", "desktop", "src-tauri", "bundles");
  const keepFiles = [
    ".gitignore",
    "README.md",
    "lucide-settings.svg",
    "runtime_lock.v1.json",
    "runtime_lock.v2.json",
  ];
  const keepBackups = new Map();
  if (fs.existsSync(destDir)) {
    for (const file of keepFiles) {
      const full = path.join(destDir, file);
      if (!fs.existsSync(full)) continue;
      keepBackups.set(file, fs.readFileSync(full));
    }
  }
  fs.rmSync(destDir, { recursive: true, force: true });
  copyRecursive(sourceDir, destDir);
  normalizePermissionsRecursive(destDir);
  for (const [file, content] of keepBackups) {
    const full = path.join(destDir, file);
    if (fs.existsSync(full)) continue;
    fs.writeFileSync(full, content);
  }
};

main();
