#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");

const LEGACY_THEME_PROBE =
  'gsettings get org.gnome.desktop.interface gtk-theme 2> /dev/null | grep -qi "dark" && GTK_THEME_VARIANT="dark" || GTK_THEME_VARIANT="light"';

const BOUNDED_THEME_PROBE = [
  "# ctx patch: linuxdeploy's unbounded gsettings probe can hang before the app binary starts.",
  'GTK_THEME_VARIANT="${APPIMAGE_GTK_THEME_VARIANT:-light}"',
  'if [[ -z "${APPIMAGE_GTK_THEME:-}" ]] && command -v timeout >/dev/null 2>&1 && command -v gsettings >/dev/null 2>&1; then',
  '    if timeout "${APPIMAGE_GTK_THEME_PROBE_TIMEOUT:-2s}" gsettings get org.gnome.desktop.interface gtk-theme 2>/dev/null | grep -qi "dark"; then',
  '        GTK_THEME_VARIANT="dark"',
  "    fi",
  "fi",
].join("\n");

const usage = () => {
  console.error("usage: patch_linuxdeploy_gtk_hook.cjs <path-to-linuxdeploy-plugin-gtk.sh>");
};

const hookPath = process.argv[2] ? path.resolve(process.argv[2]) : "";
if (!hookPath) {
  usage();
  process.exit(2);
}

let source = "";
try {
  source = fs.readFileSync(hookPath, "utf8");
} catch (err) {
  console.error(`error: failed to read linuxdeploy GTK hook at ${hookPath}: ${err.message}`);
  process.exit(1);
}

if (source.includes(BOUNDED_THEME_PROBE)) {
  process.exit(0);
}

if (!source.includes(LEGACY_THEME_PROBE)) {
  console.error(
    `error: linuxdeploy GTK hook has an unexpected theme probe; refusing to patch ${hookPath}`,
  );
  process.exit(1);
}

const patched = source.replace(LEGACY_THEME_PROBE, BOUNDED_THEME_PROBE);
fs.writeFileSync(hookPath, patched, "utf8");
