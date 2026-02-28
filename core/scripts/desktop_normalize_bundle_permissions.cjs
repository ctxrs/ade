#!/usr/bin/env node
const fs = require("fs");
const path = require("path");

const usage = () => {
  console.error("usage: node core/scripts/desktop_normalize_bundle_permissions.cjs --dir <bundle-dir>");
};

const parseArgs = (argv) => {
  let dir = "";
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    const value = argv[i + 1];
    if (flag === "--dir") {
      if (!value) throw new Error("missing value for --dir");
      dir = value;
      i += 1;
      continue;
    }
    throw new Error(`unknown argument: ${flag}`);
  }
  if (!dir) throw new Error("missing required --dir");
  return { dir };
};

const normalizePermissionsRecursive = (rootDir) => {
  if (!fs.existsSync(rootDir)) return;
  const stack = [rootDir];
  while (stack.length > 0) {
    const current = stack.pop();
    const entries = fs.readdirSync(current, { withFileTypes: true });
    for (const entry of entries) {
      const full = path.join(current, entry.name);
      if (entry.isDirectory()) {
        fs.chmodSync(full, 0o755);
        stack.push(full);
        continue;
      }
      if (entry.isFile()) {
        const mode = fs.statSync(full).mode & 0o777;
        const executable = (mode & 0o111) !== 0;
        fs.chmodSync(full, executable ? 0o755 : 0o644);
      }
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
  const rootDir = path.resolve(options.dir);
  if (!fs.existsSync(rootDir)) {
    throw new Error(`bundle dir does not exist: ${rootDir}`);
  }
  normalizePermissionsRecursive(rootDir);
};

if (require.main === module) {
  main();
}

module.exports = {
  normalizePermissionsRecursive,
  parseArgs,
};
