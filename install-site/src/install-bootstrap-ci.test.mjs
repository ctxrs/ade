import test from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, chmodSync, existsSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { renderInstallScript } from "./install-script.js";

const makeTempDir = (prefix) => mkdtempSync(path.join(tmpdir(), prefix));
const NODE_BIN = JSON.stringify(process.execPath);

const createFakeAppImage = () => `#!/bin/sh
set -eu
if [ "\${1:-}" = "--appimage-extract" ]; then
  mkdir -p squashfs-root/usr/share/icons/hicolor/512x512/apps
  printf '%s\\n' 'fake-icon-bytes' > squashfs-root/usr/share/icons/hicolor/512x512/apps/ctx.png
  exit 0
fi
exit 0
`;

const writeExecutable = (filePath, contents) => {
  writeFileSync(filePath, contents);
  chmodSync(filePath, 0o755);
};

const createExtractorScript = (stubDir) => {
  const extractorPath = path.join(stubDir, "extract-json.mjs");
  writeExecutable(
    extractorPath,
    `#!${process.execPath}
import fs from "node:fs";

const manifestPath = process.argv[2];
const keyPath = process.argv[3];
const parts = String(keyPath ?? "").split(".");
let data = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
for (const part of parts) {
  if (data && typeof data === "object" && part in data) {
    data = data[part];
    continue;
  }
  process.stdout.write("");
  process.exit(0);
}
if (["string", "number", "boolean"].includes(typeof data)) {
  process.stdout.write(String(data));
} else {
  process.stdout.write("");
}
`,
  );
  return extractorPath;
};

const installStubCommands = (stubDir) => {
  const extractorPath = createExtractorScript(stubDir);
  writeExecutable(
    path.join(stubDir, "curl"),
    `#!/bin/sh
set -eu
dest=""
url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o)
      dest="$2"
      shift 2
      ;;
    http://*|https://*|/*)
      url="$1"
      shift
      ;;
    *)
      shift
      ;;
  esac
done
[ -n "$dest" ] || exit 2
case "$url" in
  */latest.json*)
    cp "$CTX_TEST_MANIFEST_PATH" "$dest"
    ;;
  *)
    cp "$CTX_TEST_ARTIFACT_PATH" "$dest"
    ;;
esac
`,
  );
  writeExecutable(
    path.join(stubDir, "uname"),
    `#!/bin/sh
set -eu
case "$1" in
  -s) printf '%s\\n' "$CTX_TEST_UNAME_S" ;;
  -m) printf '%s\\n' "$CTX_TEST_UNAME_M" ;;
  *) exit 2 ;;
esac
`,
  );
  writeExecutable(
    path.join(stubDir, "plutil"),
    `#!/bin/sh
set -eu
key=""
manifest=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -extract)
      key="$2"
      shift 2
      ;;
    raw)
      shift
      ;;
    -o)
      shift 2
      ;;
    *)
      manifest="$1"
      shift
      ;;
  esac
done
exec ${NODE_BIN} "${extractorPath}" "$manifest" "$key"
`,
  );
  writeExecutable(
    path.join(stubDir, "hdiutil"),
    `#!/bin/sh
set -eu
cmd="$1"
shift
case "$cmd" in
  attach)
    mountpoint=""
    while [ "$#" -gt 0 ]; do
      case "$1" in
        -mountpoint)
          mountpoint="$2"
          shift 2
          ;;
        *)
          shift
          ;;
      esac
    done
    [ -n "$mountpoint" ] || exit 2
    mkdir -p "$mountpoint/ctx.app/Contents/MacOS"
    printf '%s\\n' "ctx app" > "$mountpoint/ctx.app/Contents/MacOS/ctx"
    printf '%s\\n' "plist" > "$mountpoint/ctx.app/Contents/Info.plist"
    ;;
  detach)
    exit 0
    ;;
  *)
    exit 2
    ;;
esac
`,
  );
  writeExecutable(
    path.join(stubDir, "ditto"),
    `#!${process.execPath}
import fs from "node:fs";

const [, , src, dest] = process.argv;
if (!src || !dest) process.exit(2);
const failMatch = process.env.CTX_TEST_FAIL_DITTO_DEST_MATCH ?? "";
if (failMatch && dest.includes(failMatch)) process.exit(1);
fs.rmSync(dest, { recursive: true, force: true });
fs.cpSync(src, dest, { recursive: true });
`,
  );
  for (const name of ["open", "xdg-open", "update-desktop-database"]) {
    writeExecutable(
      path.join(stubDir, name),
      `#!/bin/sh
set -eu
exit 0
`,
    );
  }
};

const sha256 = (value) => crypto.createHash("sha256").update(value).digest("hex");

const runInstaller = ({
  os,
  arch,
  manifest,
  artifactContents,
  existingMacAppFiles = null,
  extraEnv = {},
}) => {
  const sandboxDir = makeTempDir("ctx-install-bootstrap-ci-");
  const stubDir = path.join(sandboxDir, "stubs");
  mkdirSync(stubDir);
  installStubCommands(stubDir);

  const scriptPath = path.join(sandboxDir, "install.sh");
  const manifestPath = path.join(sandboxDir, "latest.json");
  const artifactPath = path.join(sandboxDir, "artifact.bin");
  const installDir = path.join(sandboxDir, "install-root");
  const binDir = path.join(sandboxDir, "bin-root");
  const xdgDataHome = path.join(sandboxDir, "xdg-data");
  const osReleasePath = path.join(sandboxDir, "os-release");

  writeFileSync(scriptPath, renderInstallScript());
  writeFileSync(manifestPath, JSON.stringify(manifest));
  writeFileSync(artifactPath, artifactContents);
  writeFileSync(osReleasePath, "ID=ubuntu\nID_LIKE=debian\n");
  chmodSync(scriptPath, 0o755);
  mkdirSync(installDir);
  mkdirSync(binDir);
  mkdirSync(xdgDataHome);
  if (existingMacAppFiles !== null) {
    for (const [relativePath, contents] of Object.entries(existingMacAppFiles)) {
      const destination = path.join(installDir, "ctx.app", relativePath);
      mkdirSync(path.dirname(destination), { recursive: true });
      writeFileSync(destination, contents);
    }
  }

  const result = spawnSync("sh", [scriptPath], {
    encoding: "utf8",
    env: {
      ...process.env,
      PATH: `${stubDir}:${process.env.PATH ?? ""}`,
      CTX_INSTALL_NO_OPEN: "1",
      CTX_INSTALL_DIR: installDir,
      CTX_BIN_DIR: binDir,
      CTX_TEST_MANIFEST_PATH: manifestPath,
      CTX_TEST_ARTIFACT_PATH: artifactPath,
      CTX_TEST_UNAME_S: os,
      CTX_TEST_UNAME_M: arch,
      CTX_INSTALL_OS_RELEASE_PATH: osReleasePath,
      XDG_DATA_HOME: xdgDataHome,
      ...extraEnv,
    },
  });

  const cleanup = () => rmSync(sandboxDir, { recursive: true, force: true });
  return { ...result, installDir, binDir, xdgDataHome, cleanup };
};

test("linux install bootstrap succeeds with a checksummed AppImage", () => {
  const artifactContents = createFakeAppImage();
  const result = runInstaller({
    os: "Linux",
    arch: "x86_64",
    artifactContents,
    manifest: {
      channel: "stable",
      latest_version: "0.0.1",
      platforms: {
        "linux-x64": {
          appimage: {
            url_path: "/download/stable/0.0.1/ctx.AppImage",
            sha256: sha256(artifactContents),
          },
        },
      },
    },
  });
  try {
    assert.equal(result.status, 0, result.stderr);
    assert.equal(existsSync(path.join(result.installDir, "ctx.AppImage")), true);
    assert.equal(existsSync(path.join(result.binDir, "ctx-desktop")), true);
    assert.equal(
      existsSync(path.join(result.xdgDataHome, "icons", "hicolor", "512x512", "apps", "ctx.png")),
      true,
    );
  } finally {
    result.cleanup();
  }
});

test("macOS install bootstrap succeeds with a checksummed dmg", () => {
  const artifactContents = "fake-dmg";
  const result = runInstaller({
    os: "Darwin",
    arch: "x86_64",
    artifactContents,
    manifest: {
      channel: "stable",
      latest_version: "0.0.1",
      platforms: {
        "macos-x64": {
          desktop: {
            url_path: "/download/stable/0.0.1/ctx.dmg",
            sha256: sha256(artifactContents),
          },
        },
      },
    },
  });
  try {
    assert.equal(result.status, 0, result.stderr);
    const installedApp = path.join(result.installDir, "ctx.app");
    assert.equal(existsSync(installedApp), true);
    assert.equal(existsSync(path.join(installedApp, "Contents", "Info.plist")), true);
    assert.match(result.stderr, /Installed ctx\.app to/);
  } finally {
    result.cleanup();
  }
});

test("macOS upgrade preserves the existing app when staging the replacement fails", () => {
  const artifactContents = "fake-dmg";
  const existingInfoPlist = "existing plist";
  const result = runInstaller({
    os: "Darwin",
    arch: "x86_64",
    artifactContents,
    existingMacAppFiles: {
      "Contents/Info.plist": existingInfoPlist,
      "Contents/MacOS/ctx": "existing binary",
    },
    extraEnv: {
      CTX_TEST_FAIL_DITTO_DEST_MATCH: ".ctx-stage.",
    },
    manifest: {
      channel: "stable",
      latest_version: "0.0.2",
      platforms: {
        "macos-x64": {
          desktop: {
            url_path: "/download/stable/0.0.2/ctx.dmg",
            sha256: sha256(artifactContents),
          },
        },
      },
    },
  });
  try {
    assert.notEqual(result.status, 0);
    const installedApp = path.join(result.installDir, "ctx.app");
    assert.equal(readFileSync(path.join(installedApp, "Contents", "Info.plist"), "utf8").trim(), existingInfoPlist);
    assert.equal(existsSync(path.join(installedApp, "Contents", "MacOS", "ctx")), true);
  } finally {
    result.cleanup();
  }
});
