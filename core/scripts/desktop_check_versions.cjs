const fs = require("fs");
const path = require("path");

const coreRoot = path.resolve(__dirname, "..");

const desktopPkgJsonPath = path.join(coreRoot, "apps", "desktop", "package.json");
const tauriConfPath = path.join(coreRoot, "apps", "desktop", "src-tauri", "tauri.conf.json");
const tauriCargoTomlPath = path.join(coreRoot, "apps", "desktop", "src-tauri", "Cargo.toml");

const readJson = (p) => JSON.parse(fs.readFileSync(p, "utf8"));

const readCargoVersion = (p) => {
  const text = fs.readFileSync(p, "utf8");
  const match = text.match(/^version\\s*=\\s*\"([^\"]+)\"\\s*$/m);
  if (!match) {
    throw new Error(`failed to find Cargo.toml version in ${p}`);
  }
  return match[1];
};

const main = () => {
  const desktopVersion = readJson(desktopPkgJsonPath).version;
  const tauriVersion = readJson(tauriConfPath).package?.version;
  const cargoVersion = readCargoVersion(tauriCargoTomlPath);

  const problems = [];
  if (!desktopVersion) {
    problems.push(`missing desktop package.json version (${desktopPkgJsonPath})`);
  }
  if (!tauriVersion) {
    problems.push(`missing tauri.conf.json package.version (${tauriConfPath})`);
  }
  if (desktopVersion !== tauriVersion) {
    problems.push(
      `version mismatch: apps/desktop/package.json=${desktopVersion} src-tauri/tauri.conf.json=${tauriVersion}`,
    );
  }
  if (desktopVersion !== cargoVersion) {
    problems.push(
      `version mismatch: apps/desktop/package.json=${desktopVersion} src-tauri/Cargo.toml=${cargoVersion}`,
    );
  }

  if (problems.length) {
    for (const p of problems) console.error(`error: ${p}`);
    process.exit(1);
  }

  console.log(`desktop_check_versions: OK (${desktopVersion})`);
};

main();

