#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TAURI_DIR="$(cd "${SCRIPT_DIR}/../src-tauri" && pwd)"
CAPABILITY_FILE="${TAURI_DIR}/capabilities/default.json"

node - "${CAPABILITY_FILE}" <<'NODE'
const fs = require("fs");

const capabilityPath = process.argv[2];
const capability = JSON.parse(fs.readFileSync(capabilityPath, "utf8"));

const expected = [
  "core:app:allow-version",
  "core:event:allow-emit",
  "core:event:allow-listen",
  "core:event:allow-unlisten",
  "core:webview:allow-webview-position",
  "core:webview:allow-webview-size",
  "core:window:allow-close",
  "core:window:allow-inner-position",
  "core:window:allow-inner-size",
  "core:window:allow-internal-toggle-maximize",
  "core:window:allow-is-maximized",
  "core:window:allow-minimize",
  "core:window:allow-outer-position",
  "core:window:allow-outer-size",
  "core:window:allow-scale-factor",
  "core:window:allow-start-dragging",
  "core:window:allow-toggle-maximize",
  "os:allow-platform",
];

const actual = capability.permissions;
if (!Array.isArray(actual)) {
  throw new Error("desktop default capability permissions must be an array");
}

const forbidden = [
  "core:default",
  "os:default",
  "shell:default",
  "dialog:default",
  "deep-link:default",
];
const forbiddenPresent = actual.filter((permission) => forbidden.includes(permission));
if (forbiddenPresent.length > 0) {
  throw new Error(`desktop renderer capability exposes broad permissions: ${forbiddenPresent.join(", ")}`);
}

const missing = expected.filter((permission) => !actual.includes(permission));
const extra = actual.filter((permission) => !expected.includes(permission));
if (missing.length > 0 || extra.length > 0) {
  throw new Error(
    [
      "desktop renderer capability permissions drifted",
      missing.length ? `missing: ${missing.join(", ")}` : null,
      extra.length ? `extra: ${extra.join(", ")}` : null,
    ].filter(Boolean).join("\n"),
  );
}

for (let index = 0; index < expected.length; index += 1) {
  if (actual[index] !== expected[index]) {
    throw new Error("desktop renderer capability permissions must stay in the reviewed order");
  }
}
NODE
