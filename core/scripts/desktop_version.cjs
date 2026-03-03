#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const resolveCoreRoot = () => path.resolve(__dirname, "..");

const readDesktopVersion = (coreRoot = resolveCoreRoot()) => {
  const desktopPkgPath = path.join(coreRoot, "apps", "desktop", "package.json");
  const raw = fs.readFileSync(desktopPkgPath, "utf8");
  const parsed = JSON.parse(raw);
  const version = String(parsed?.version ?? "").trim();
  if (!version) {
    throw new Error(`missing desktop version in ${desktopPkgPath}`);
  }
  return version;
};

if (require.main === module) {
  try {
    process.stdout.write(readDesktopVersion());
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    console.error(`desktop_version failed: ${detail}`);
    process.exit(1);
  }
}

module.exports = {
  readDesktopVersion,
};
