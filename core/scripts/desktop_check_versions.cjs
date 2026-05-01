const fs = require("fs");
const path = require("path");

const coreRoot = path.resolve(__dirname, "..");

const desktopPkgJsonPath = path.join(coreRoot, "apps", "desktop", "package.json");
const tauriConfPath = path.join(coreRoot, "apps", "desktop", "src-tauri", "tauri.conf.json");
const tauriCargoTomlPath = path.join(coreRoot, "apps", "desktop", "src-tauri", "Cargo.toml");
const daemonCargoTomlPath = path.join(coreRoot, "crates", "ctx-http", "Cargo.toml");
const daemonBazelBuildPath = path.join(coreRoot, "crates", "ctx-http", "BUILD.bazel");

const readJson = (p) => JSON.parse(fs.readFileSync(p, "utf8"));

const readCargoVersion = (p) => {
  const text = fs.readFileSync(p, "utf8");

  // Prefer `[package]` -> `version = "..."` to avoid matching unrelated version fields.
  const lines = text.split(/\n/);
  let inPackage = false;
  for (const rawLine of lines) {
    const line = rawLine.replace(/^\uFEFF/, "");
    const section = line.match(/^\s*\[([^\]]+)\]\s*$/);
    if (section) {
      inPackage = section[1].trim() === "package";
      continue;
    }
    if (!inPackage) continue;

    const match = line.match(/^\s*version\s*=\s*"([^"]+)"\s*(?:#.*)?$/);
    if (match) return match[1];
  }

  // Fall back to a looser scan in case formatting is unusual.
  const match = text.match(/^\s*version\s*=\s*"([^"]+)"\s*(?:#.*)?$/m);
  if (match) return match[1];

  throw new Error(`failed to find Cargo.toml version in ${p}`);
};

const readBazelCargoPkgVersion = (p) => {
  const text = fs.readFileSync(p, "utf8");
  const match = text.match(/"CARGO_PKG_VERSION"\s*:\s*"([^"]+)"/);
  if (!match) {
    throw new Error(`failed to find CARGO_PKG_VERSION in ${p}`);
  }
  return match[1];
};

const main = () => {
  const desktopVersion = readJson(desktopPkgJsonPath).version;
  const tauriConf = readJson(tauriConfPath);
  const tauriVersion = tauriConf.package?.version ?? tauriConf.version;
  const cargoVersion = readCargoVersion(tauriCargoTomlPath);
  const daemonCargoVersion = readCargoVersion(daemonCargoTomlPath);
  const daemonBazelVersion = readBazelCargoPkgVersion(daemonBazelBuildPath);

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
  if (desktopVersion !== daemonCargoVersion) {
    problems.push(
      `version mismatch: apps/desktop/package.json=${desktopVersion} crates/ctx-http/Cargo.toml=${daemonCargoVersion}`,
    );
  }
  if (desktopVersion !== daemonBazelVersion) {
    problems.push(
      `version mismatch: apps/desktop/package.json=${desktopVersion} crates/ctx-http/BUILD.bazel CARGO_PKG_VERSION=${daemonBazelVersion}`,
    );
  }

  if (problems.length) {
    for (const p of problems) console.error(`error: ${p}`);
    process.exit(1);
  }

  console.log(`desktop_check_versions: OK (${desktopVersion})`);
};

main();
