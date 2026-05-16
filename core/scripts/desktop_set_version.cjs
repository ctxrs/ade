#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");

const desktopPkgJsonPath = path.join(coreRoot, "apps", "desktop", "package.json");
const tauriConfPath = path.join(coreRoot, "apps", "desktop", "src-tauri", "tauri.conf.json");
const tauriCargoTomlPath = path.join(coreRoot, "apps", "desktop", "src-tauri", "Cargo.toml");
const daemonCargoTomlPath = path.join(coreRoot, "crates", "ctx-http", "Cargo.toml");
const daemonBazelBuildPath = path.join(coreRoot, "crates", "ctx-http", "BUILD.bazel");
const cargoLockPath = path.join(coreRoot, "Cargo.lock");

const VERSION_RE = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;

const readJson = (filePath) => JSON.parse(fs.readFileSync(filePath, "utf8"));

const writeIfChanged = (filePath, nextText) => {
  const prevText = fs.readFileSync(filePath, "utf8");
  if (prevText !== nextText) {
    fs.writeFileSync(filePath, nextText, "utf8");
  }
};

const replaceFirst = (text, pattern, replacement) => {
  if (!pattern.test(text)) return text;
  pattern.lastIndex = 0;
  return text.replace(pattern, replacement);
};

const assertValidVersion = (nextVersion) => {
  const normalized = String(nextVersion || "").trim();
  if (!normalized || !VERSION_RE.test(normalized)) {
    throw new Error(`invalid semver version '${nextVersion || ""}'`);
  }
  return normalized;
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

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index].replace(/^\uFEFF/, "");
    const section = line.match(/^\s*\[([^\]]+)\]\s*$/);
    if (section) {
      inPackage = section[1].trim() === "package";
      continue;
    }
    if (!inPackage) continue;
    if (/^\s*version\s*=\s*"[^"]+"\s*(?:#.*)?$/.test(line)) {
      const indent = (line.match(/^\s*/) || [""])[0];
      lines[index] = `${indent}version = "${nextVersion}"`;
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

const updateBazelCargoPkgVersion = (buildPath, nextVersion) => {
  const text = fs.readFileSync(buildPath, "utf8");
  const match = text.match(/"CARGO_PKG_VERSION"\s*:\s*"([^"]+)"/);
  if (!match) {
    throw new Error(`failed to update CARGO_PKG_VERSION in ${buildPath}`);
  }
  if (match[1] === nextVersion) {
    return;
  }
  const next = replaceFirst(
    text,
    /("CARGO_PKG_VERSION"\s*:\s*")([^"]+)(")/,
    (_, prefix, _old, suffix) => `${prefix}${nextVersion}${suffix}`,
  );
  writeIfChanged(buildPath, next);
};

const updateCargoLockPackageVersion = (lockPath, packageName, nextVersion) => {
  const text = fs.readFileSync(lockPath, "utf8");
  const escapedPackageName = packageName.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const pattern = new RegExp(`(\\[\\[package\\]\\]\\nname = "${escapedPackageName}"\\nversion = ")([^"]+)(")`);
  const match = text.match(pattern);
  if (!match) {
    throw new Error(`failed to update ${packageName} version in ${lockPath}`);
  }
  if (match[2] === nextVersion) {
    return;
  }
  const next = replaceFirst(
    text,
    pattern,
    (_, prefix, _old, suffix) => `${prefix}${nextVersion}${suffix}`,
  );
  writeIfChanged(lockPath, next);
};

const setDesktopVersion = (nextVersion, { root = coreRoot } = {}) => {
  const normalizedVersion = assertValidVersion(nextVersion);
  const resolvedDesktopPkgJsonPath = path.join(root, "apps", "desktop", "package.json");
  const resolvedTauriConfPath = path.join(root, "apps", "desktop", "src-tauri", "tauri.conf.json");
  const resolvedTauriCargoTomlPath = path.join(root, "apps", "desktop", "src-tauri", "Cargo.toml");
  const resolvedDaemonCargoTomlPath = path.join(root, "crates", "ctx-http", "Cargo.toml");
  const resolvedDaemonBazelBuildPath = path.join(root, "crates", "ctx-http", "BUILD.bazel");
  const resolvedCargoLockPath = path.join(root, "Cargo.lock");

  const desktopPkg = readJson(resolvedDesktopPkgJsonPath);
  if (desktopPkg.version !== normalizedVersion) {
    desktopPkg.version = normalizedVersion;
    writeIfChanged(resolvedDesktopPkgJsonPath, `${JSON.stringify(desktopPkg, null, 2)}\n`);
  }

  updateJsonVersionField(resolvedTauriConfPath, normalizedVersion);
  updateTauriPackageVersionIfPresent(resolvedTauriConfPath, normalizedVersion);
  updateCargoPackageVersion(resolvedTauriCargoTomlPath, normalizedVersion);
  updateCargoPackageVersion(resolvedDaemonCargoTomlPath, normalizedVersion);
  updateBazelCargoPkgVersion(resolvedDaemonBazelBuildPath, normalizedVersion);
  updateCargoLockPackageVersion(resolvedCargoLockPath, "ctx-http", normalizedVersion);

  return {
    root,
    version: normalizedVersion,
  };
};

const main = () => {
  const nextVersionRaw = process.argv[2];
  const nextVersion = assertValidVersion(nextVersionRaw);
  const result = setDesktopVersion(nextVersion);
  console.log(
    `desktop_set_version: updated desktop+daemon to ${result.version} (${desktopPkgJsonPath}, ${tauriConfPath}, ${tauriCargoTomlPath}, ${daemonCargoTomlPath}, ${daemonBazelBuildPath}, ${cargoLockPath})`,
  );
};

if (require.main === module) {
  try {
    main();
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    console.error(`desktop_set_version failed: ${detail}`);
    process.exit(1);
  }
}

module.exports = {
  VERSION_RE,
  assertValidVersion,
  setDesktopVersion,
  updateCargoLockPackageVersion,
};
