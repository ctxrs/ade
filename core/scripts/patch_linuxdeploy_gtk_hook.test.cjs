const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const PATCH_SCRIPT = path.resolve(__dirname, "patch_linuxdeploy_gtk_hook.cjs");

const legacyHook = () => `#! /usr/bin/env bash

gsettings get org.gnome.desktop.interface gtk-theme 2> /dev/null | grep -qi "dark" && GTK_THEME_VARIANT="dark" || GTK_THEME_VARIANT="light"
APPIMAGE_GTK_THEME="\${APPIMAGE_GTK_THEME:-"Adwaita:$GTK_THEME_VARIANT"}" # Allow user to override theme (discouraged)

export APPDIR="\${APPDIR:-"$(dirname "$(realpath "$0")")"}" # Workaround to run extracted AppImage
export GTK_THEME="$APPIMAGE_GTK_THEME" # Custom themes are broken
`;

const mkTempDir = () => fs.mkdtempSync(path.join(os.tmpdir(), "ctx-linuxdeploy-gtk-hook-"));

test("patches linuxdeploy GTK theme probe to be bounded and idempotent", () => {
  const tmp = mkTempDir();
  try {
    const hookPath = path.join(tmp, "linuxdeploy-plugin-gtk.sh");
    fs.writeFileSync(hookPath, legacyHook(), { mode: 0o755 });

    const first = spawnSync(process.execPath, [PATCH_SCRIPT, hookPath], {
      encoding: "utf8",
    });
    assert.equal(first.status, 0, first.stderr);

    const patched = fs.readFileSync(hookPath, "utf8");
    assert.doesNotMatch(patched, /gsettings get .* \| grep -qi "dark" &&/);
    assert.match(patched, /APPIMAGE_GTK_PROBE_TIMEOUT|APPIMAGE_GTK_THEME_PROBE_TIMEOUT/);
    assert.match(patched, /timeout "\$\{APPIMAGE_GTK_THEME_PROBE_TIMEOUT:-2s\}" gsettings get/);
    assert.match(patched, /APPIMAGE_GTK_THEME="\$\{APPIMAGE_GTK_THEME:-"Adwaita:\$GTK_THEME_VARIANT"\}"/);

    const second = spawnSync(process.execPath, [PATCH_SCRIPT, hookPath], {
      encoding: "utf8",
    });
    assert.equal(second.status, 0, second.stderr);
    assert.equal(fs.readFileSync(hookPath, "utf8"), patched);
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test("patched AppRun reaches the wrapped binary when gsettings hangs", () => {
  const tmp = mkTempDir();
  try {
    const appDir = path.join(tmp, "AppDir");
    const hooksDir = path.join(appDir, "apprun-hooks");
    const fakeBin = path.join(tmp, "bin");
    fs.mkdirSync(hooksDir, { recursive: true });
    fs.mkdirSync(fakeBin, { recursive: true });

    const hookPath = path.join(hooksDir, "linuxdeploy-plugin-gtk.sh");
    fs.writeFileSync(hookPath, legacyHook(), { mode: 0o755 });

    const appRunPath = path.join(appDir, "AppRun");
    fs.writeFileSync(
      appRunPath,
      `#! /usr/bin/env bash
set -e
this_dir="$(cd "$(dirname "$0")" && pwd)"
source "$this_dir"/apprun-hooks/"linuxdeploy-plugin-gtk.sh"
exec "$this_dir"/AppRun.wrapped "$@"
`,
      { mode: 0o755 },
    );

    const markerPath = path.join(tmp, "wrapped-ran");
    fs.writeFileSync(
      path.join(appDir, "AppRun.wrapped"),
      `#!/bin/sh
printf 'wrapped\\n' > '${markerPath}'
`,
      { mode: 0o755 },
    );
    fs.writeFileSync(
      path.join(fakeBin, "gsettings"),
      "#!/bin/sh\nsleep 5\n",
      { mode: 0o755 },
    );

    const patch = spawnSync(process.execPath, [PATCH_SCRIPT, hookPath], {
      encoding: "utf8",
    });
    assert.equal(patch.status, 0, patch.stderr);

    const run = spawnSync(appRunPath, [], {
      encoding: "utf8",
      env: {
        ...process.env,
        PATH: `${fakeBin}${path.delimiter}${process.env.PATH || ""}`,
        APPIMAGE_GTK_THEME_PROBE_TIMEOUT: "0.1s",
      },
      timeout: 1500,
    });
    assert.equal(run.signal, null, run.stderr);
    assert.equal(run.status, 0, run.stderr);
    assert.equal(fs.readFileSync(markerPath, "utf8"), "wrapped\n");
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});
