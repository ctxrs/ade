#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const { resolveLaunchMode } = require("./desktop_mode.cjs");

const coreRoot = path.resolve(__dirname, "..");
const rootPackageJson = JSON.parse(
  fs.readFileSync(path.join(coreRoot, "package.json"), "utf8"),
);
const desktopPackageJson = JSON.parse(
  fs.readFileSync(path.join(coreRoot, "apps", "desktop", "package.json"), "utf8"),
);

test("mode contract commands are present", () => {
  const scripts = rootPackageJson.scripts || {};
  const required = [
    "desktop-prod",
    "desktop-staging",
    "desktop-dev",
    "desktop-dev-override",
    "desktop-dev-source",
    "daemon-web-dev",
  ];
  for (const id of required) {
    assert.ok(
      typeof scripts[id] === "string" && scripts[id].trim().length > 0,
      `missing package script '${id}'`,
    );
  }
});

test("desktop prep scripts inject desktop version into web build", () => {
  const scripts = rootPackageJson.scripts || {};
  assert.equal(scripts["desktop:prep"], "node scripts/desktop_prepare.cjs --mode debug-build");
  assert.equal(scripts["desktop:prep:release"], "node scripts/desktop_prepare.cjs --mode release-build");
});

test("desktop prep scripts default to thin bundle sync", () => {
  const scripts = rootPackageJson.scripts || {};
  assert.equal(scripts["desktop:prep:dev"], "node scripts/desktop_prepare.cjs --mode dev");
});

test("desktop app scripts route tauri through the guarded entrypoint", () => {
  const scripts = desktopPackageJson.scripts || {};
  assert.equal(scripts.dev, "node ../../scripts/desktop_tauri_entry.cjs dev");
  assert.equal(scripts.build, "node ../../scripts/desktop_tauri_entry.cjs build");
});

test("tracked desktop bundle manifest remains thin by default", () => {
  const manifestPath = path.join(
    coreRoot,
    "apps",
    "desktop",
    "src-tauri",
    "bundles",
    "manifest.json",
  );
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  for (const key of ["providers", "runtimes", "images", "daemons"]) {
    assert.ok(Array.isArray(manifest[key]), `manifest.${key} must be an array`);
    assert.equal(
      manifest[key].length,
      0,
      `manifest.${key} must be empty for thin manifest default policy`,
    );
  }
});

test("desktop mode defaults to dev/parity/desktop", () => {
  const mode = resolveLaunchMode({});
  assert.deepEqual(mode, {
    channel: "dev",
    profile: "parity",
    surface: "desktop",
  });
});

test("desktop mode parser validates expected enum values", () => {
  assert.throws(
    () => resolveLaunchMode({ channel: "qa" }),
    /invalid CTX_DESKTOP_CHANNEL/,
  );
  assert.throws(
    () => resolveLaunchMode({ profile: "legacy" }),
    /invalid CTX_RUNTIME_PROFILE/,
  );
  assert.throws(
    () => resolveLaunchMode({ surface: "desktop-local" }),
    /invalid CTX_LAUNCH_SURFACE/,
  );
});

test("desktop mode parser accepts explicit composed mode", () => {
  const mode = resolveLaunchMode({
    channel: "staging",
    profile: "override",
    surface: "daemon-web",
  });
  assert.deepEqual(mode, {
    channel: "staging",
    profile: "override",
    surface: "daemon-web",
  });
});
