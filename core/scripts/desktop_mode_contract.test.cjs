#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const { resolveLaunchMode } = require("./desktop_mode.cjs");

const coreRoot = path.resolve(__dirname, "..");
const packageJson = JSON.parse(
  fs.readFileSync(path.join(coreRoot, "package.json"), "utf8"),
);

test("mode contract commands are present", () => {
  const scripts = packageJson.scripts || {};
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

