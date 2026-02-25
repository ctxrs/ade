const fs = require("fs");
const path = require("path");

const coreRoot = path.resolve(__dirname, "..");

const desktopPkgJsonPath = path.join(coreRoot, "apps", "desktop", "package.json");
const tauriConfPath = path.join(coreRoot, "apps", "desktop", "src-tauri", "tauri.conf.json");
const tauriCargoTomlPath = path.join(coreRoot, "apps", "desktop", "src-tauri", "Cargo.toml");
const daemonCargoTomlPath = path.join(coreRoot, "crates", "ctx-http", "Cargo.toml");

const VERSION_RE = /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/;

const readJson = (p) => JSON.parse(fs.readFileSync(p, "utf8"));
const writeIfChanged = (p, nextText) => {
  const prevText = fs.readFileSync(p, "utf8");
  if (prevText !== nextText) {
    fs.writeFileSync(p, nextText, "utf8");
  }
};

const replaceFirst = (text, pattern, replacement) => {
  if (!pattern.test(text)) return text;
  pattern.lastIndex = 0;
  return text.replace(pattern, replacement);
};

const updateJsonVersionField = (jsonPath, nextVersion) => {
  const text = fs.readFileSync(jsonPath, "utf8");
  const match = text.match(/("version"\s*:\s*")([^"]+)(")/);
  if (!match) {
    throw new Error(`failed to update top-level version in ${jsonPath}`);
  }
  if (match[2] === nextVersion) {
    return;
  }
  const next = replaceFirst(
    text,
    /("version"\s*:\s*")([^"]+)(")/,
    (_, prefix, _old, suffix) => `${prefix}${nextVersion}${suffix}`,
  );
  writeIfChanged(jsonPath, next);
};

const updateTauriPackageVersionIfPresent = (jsonPath, nextVersion) => {
  const text = fs.readFileSync(jsonPath, "utf8");
  const packageBlockMatch = text.match(/"package"\s*:\s*\{[\s\S]*?\}/m);
  if (!packageBlockMatch) return;
  const block = packageBlockMatch[0];
  if (!/"version"\s*:\s*"/.test(block)) return;
  const nextBlock = replaceFirst(
    block,
    /("version"\s*:\s*")([^"]+)(")/,
    (_, prefix, _old, suffix) => `${prefix}${nextVersion}${suffix}`,
  );
  const next = text.replace(block, nextBlock);
  writeIfChanged(jsonPath, next);
};

const updateCargoPackageVersion = (cargoPath, nextVersion) => {
  const text = fs.readFileSync(cargoPath, "utf8");
  const hasTrailingNewline = text.endsWith("\n");
  const lines = hasTrailingNewline ? text.slice(0, -1).split("\n") : text.split("\n");
  let inPackage = false;
  let updated = false;

  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i].replace(/^\uFEFF/, "");
    const section = line.match(/^\s*\[([^\]]+)\]\s*$/);
    if (section) {
      inPackage = section[1].trim() === "package";
      continue;
    }
    if (!inPackage) continue;
    if (/^\s*version\s*=\s*"[^"]+"\s*(?:#.*)?$/.test(line)) {
      const indent = (line.match(/^\s*/) || [""])[0];
      lines[i] = `${indent}version = "${nextVersion}"`;
      updated = true;
      break;
    }
  }

  if (!updated) {
    throw new Error(`failed to update [package] version in ${cargoPath}`);
  }
  const nextText = `${lines.join("\n")}${hasTrailingNewline ? "\n" : ""}`;
  writeIfChanged(cargoPath, nextText);
};

const main = () => {
  const nextVersionRaw = process.argv[2];
  const nextVersion = (nextVersionRaw || "").trim();
  if (!nextVersion || !VERSION_RE.test(nextVersion)) {
    console.error("usage: node core/scripts/desktop_set_version.cjs <semver>");
    process.exit(2);
  }

  const desktopPkg = readJson(desktopPkgJsonPath);
  if (desktopPkg.version !== nextVersion) {
    desktopPkg.version = nextVersion;
    writeIfChanged(desktopPkgJsonPath, `${JSON.stringify(desktopPkg, null, 2)}\n`);
  }

  updateJsonVersionField(tauriConfPath, nextVersion);
  updateTauriPackageVersionIfPresent(tauriConfPath, nextVersion);

  updateCargoPackageVersion(tauriCargoTomlPath, nextVersion);
  updateCargoPackageVersion(daemonCargoTomlPath, nextVersion);

  console.log(
    `desktop_set_version: updated desktop+daemon to ${nextVersion} (${desktopPkgJsonPath}, ${tauriConfPath}, ${tauriCargoTomlPath}, ${daemonCargoTomlPath})`,
  );
};

main();
