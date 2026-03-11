import test from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, chmodSync, existsSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { renderInstallScript } from "./install-script.js";

const makeTempDir = (prefix) => mkdtempSync(path.join(tmpdir(), prefix));

const writeExecutable = (filePath, contents) => {
  writeFileSync(filePath, contents);
  chmodSync(filePath, 0o755);
};

const createExtractorScript = (stubDir) => {
  const extractorPath = path.join(stubDir, "extract-json.mjs");
  writeExecutable(
    extractorPath,
    `#!/usr/bin/env node
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
    path.join(stubDir, "python3"),
    `#!/bin/sh
set -eu
exec node "${extractorPath}" "$2" "$3"
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
exec node "${extractorPath}" "$manifest" "$key"
`,
  );
  for (const name of ["hdiutil", "ditto", "open", "xdg-open"]) {
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
  installDirName = "install-root",
  binDirName = "bin-root",
}) => {
  const sandboxDir = makeTempDir("ctx-install-script-");
  const stubDir = path.join(sandboxDir, "stubs");
  mkdirSync(stubDir);
  installStubCommands(stubDir);

  const scriptPath = path.join(sandboxDir, "install.sh");
  const manifestPath = path.join(sandboxDir, "latest.json");
  const artifactPath = path.join(sandboxDir, "artifact.bin");
  const installDir = path.join(sandboxDir, installDirName);
  const binDir = path.join(sandboxDir, binDirName);

  writeFileSync(scriptPath, renderInstallScript());
  writeFileSync(manifestPath, JSON.stringify(manifest));
  writeFileSync(artifactPath, artifactContents);
  chmodSync(scriptPath, 0o755);
  mkdirSync(installDir);
  mkdirSync(binDir);

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
    },
  });

  const cleanup = () => rmSync(sandboxDir, { recursive: true, force: true });
  return { ...result, installDir, binDir, cleanup };
};

test("renderInstallScript emits a bootstrap script with stable defaults", () => {
  const script = renderInstallScript();
  assert.match(script, /^#!\/bin\/sh/m);
  assert.match(script, /set -eu/);
  assert.match(script, /install_macos/);
  assert.match(script, /install_linux/);
  assert.match(script, /install_windows/);
  assert.match(script, /windows support is coming soon!/);
  assert.match(script, /https:\/\/api\.ctx\.rs\/functions\/v1/);
  assert.match(script, /channel="\$\{CTX_CHANNEL:-stable\}"/);
  assert.match(script, /download_id="\$\{CTX_DOWNLOAD_ID:-\}"/);
});

test("renderInstallScript includes release resolution, checksum verify, and app launch", () => {
  const script = renderInstallScript();
  assert.match(script, /releases\/\$channel\/latest\.json/);
  assert.match(script, /append_release_attribution/);
  assert.match(script, /ctx_download_id/);
  assert.match(script, /plutil -extract/);
  assert.match(script, /sha256sum/);
  assert.match(script, /shasum -a 256/);
  assert.match(script, /python3 - "\$manifest_json" "\$key"/);
  assert.match(script, /fail "manifest missing sha256 for selected artifact"/);
  assert.doesNotMatch(script, /skipping checksum verification/);
  assert.match(script, /hdiutil attach/);
  assert.match(script, /ditto "\$app_src" "\$target_app"/);
  assert.match(script, /open "\$target_app"/);
  assert.match(script, /CTX_DESKTOP_START_PATH="\$start_path"/);
  assert.match(script, /ctx\.AppImage/);
  assert.match(script, /ctx-desktop/);
  assert.match(script, /first_open_start_path/);
  assert.match(script, /CTX_INSTALL_DIR/);
});

test("renderInstallScript allows overriding function base and channel", () => {
  const script = renderInstallScript({
    functionsBase: "https://example.test/functions/v1/",
    channel: "rc",
    downloadId: "dl_123",
    referrerDomain: "ctx.rs",
    utmSource: "twitter",
    utmMedium: "social",
    utmCampaign: "public-beta",
  });
  assert.match(script, /functions_base="\$\{CTX_FUNCTIONS_BASE:-https:\/\/example\.test\/functions\/v1\}"/);
  assert.match(script, /channel="\$\{CTX_CHANNEL:-rc\}"/);
  assert.match(script, /download_id="\$\{CTX_DOWNLOAD_ID:-dl_123\}"/);
  assert.match(script, /referrer_domain="\$\{CTX_INSTALL_REFERRER_DOMAIN:-ctx\.rs\}"/);
  assert.match(script, /utm_source="\$\{CTX_INSTALL_UTM_SOURCE:-twitter\}"/);
  assert.match(script, /utm_medium="\$\{CTX_INSTALL_UTM_MEDIUM:-social\}"/);
  assert.match(script, /utm_campaign="\$\{CTX_INSTALL_UTM_CAMPAIGN:-public-beta\}"/);
});

test("linux install hard-fails when manifest omits sha256", () => {
  const result = runInstaller({
    os: "Linux",
    arch: "x86_64",
    artifactContents: "fake-appimage",
    manifest: {
      channel: "stable",
      latest_version: "0.0.1",
      platforms: {
        "linux-x64": {
          desktop: {
            url_path: "/download/stable/0.0.1/ctx.AppImage",
          },
        },
      },
    },
  });
  try {
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /error: manifest missing sha256 for selected artifact/);
    assert.equal(existsSync(path.join(result.installDir, "ctx.AppImage")), false);
  } finally {
    result.cleanup();
  }
});

test("macOS install hard-fails when manifest omits sha256", () => {
  const result = runInstaller({
    os: "Darwin",
    arch: "x86_64",
    artifactContents: "fake-dmg",
    manifest: {
      channel: "stable",
      latest_version: "0.0.1",
      platforms: {
        "macos-x64": {
          desktop: {
            url_path: "/download/stable/0.0.1/ctx.dmg",
          },
        },
      },
    },
  });
  try {
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /error: manifest missing sha256 for selected artifact/);
  } finally {
    result.cleanup();
  }
});

test("linux install succeeds when manifest includes sha256", () => {
  const artifactContents = "fake-appimage-with-sha";
  const result = runInstaller({
    os: "Linux",
    arch: "x86_64",
    artifactContents,
    manifest: {
      channel: "stable",
      latest_version: "0.0.1",
      platforms: {
        "linux-x64": {
          desktop: {
            url_path: "/download/stable/0.0.1/ctx.AppImage",
            sha256: sha256(artifactContents),
          },
        },
      },
    },
  });
  try {
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stderr, /Verified artifact sha256/);
    assert.equal(existsSync(path.join(result.installDir, "ctx.AppImage")), true);
    assert.equal(existsSync(path.join(result.binDir, "ctx-desktop")), true);
  } finally {
    result.cleanup();
  }
});
