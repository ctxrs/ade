const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { copySidecarBinary } = require("./desktop_sync_resources.cjs");

test("copySidecarBinary supports packaging a daemon under a distinct bundle name", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-sidecar-copy-"));
  const sourceDir = path.join(root, "source");
  const destDir = path.join(root, "dest");
  const binExt = process.platform === "win32" ? ".exe" : "";
  const sourcePath = path.join(sourceDir, `ctx${binExt}`);

  try {
    fs.mkdirSync(sourceDir, { recursive: true });
    fs.mkdirSync(destDir, { recursive: true });
    fs.writeFileSync(sourcePath, "fake-daemon-binary", "utf8");

    const { dest, destTarget } = copySidecarBinary({
      sourceDir,
      destDir,
      sourceName: "ctx",
      destName: "ctx-daemon",
      targetTriple: "x86_64-unknown-linux-gnu",
      binExtOverride: binExt,
    });

    assert.equal(path.basename(dest), `ctx-daemon${binExt}`);
    assert.equal(path.basename(destTarget), `ctx-daemon-x86_64-unknown-linux-gnu${binExt}`);
    assert.equal(fs.existsSync(dest), true);
    assert.equal(fs.existsSync(destTarget), true);
    assert.equal(fs.existsSync(path.join(destDir, `ctx${binExt}`)), false);
    assert.equal(fs.readFileSync(dest, "utf8"), "fake-daemon-binary");
    assert.equal(fs.readFileSync(destTarget, "utf8"), "fake-daemon-binary");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("copySidecarBinary codesigns the AVF helper on darwin", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-sidecar-sign-"));
  const sourceDir = path.join(root, "source");
  const destDir = path.join(root, "dest");
  const entitlementsPath = path.join(root, "ctx-avf-linux-helper.entitlements");
  const calls = [];

  try {
    fs.mkdirSync(sourceDir, { recursive: true });
    fs.mkdirSync(destDir, { recursive: true });
    fs.writeFileSync(path.join(sourceDir, "ctx-avf-linux-helper"), "fake-helper", "utf8");
    fs.writeFileSync(entitlementsPath, "<plist />", "utf8");

    const { dest, destTarget } = copySidecarBinary({
      sourceDir,
      destDir,
      sourceName: "ctx-avf-linux-helper",
      targetTriple: "aarch64-apple-darwin",
      platform: "darwin",
      helperEntitlementsPath: entitlementsPath,
      spawnSyncImpl: (cmd, args) => {
        calls.push({
          cmd,
          args,
          signedPath: args.at(-1),
          destExistsBeforeCall: fs.existsSync(path.join(destDir, "ctx-avf-linux-helper")),
          destTargetExistsBeforeCall: fs.existsSync(
            path.join(destDir, "ctx-avf-linux-helper-aarch64-apple-darwin"),
          ),
        });
        return { status: 0 };
      },
    });

    assert.equal(fs.existsSync(dest), true);
    assert.equal(fs.existsSync(destTarget), true);
    assert.equal(calls.length, 4);
    assert.deepEqual(
      calls.map((entry) => entry.args.slice(0, -1)),
      [
        ["--force", "--sign", "-", "--entitlements", entitlementsPath],
        ["--force", "--sign", "-", "--entitlements", entitlementsPath],
        ["--verify", "--verbose=1"],
        ["--verify", "--verbose=1"],
      ],
    );
    assert.equal(calls[0].cmd, "/usr/bin/codesign");
    assert.equal(calls[1].cmd, "/usr/bin/codesign");
    assert.equal(calls[2].cmd, "/usr/bin/codesign");
    assert.equal(calls[3].cmd, "/usr/bin/codesign");
    assert.notEqual(calls[0].signedPath, dest);
    assert.notEqual(calls[1].signedPath, destTarget);
    assert.equal(calls[0].destExistsBeforeCall, false);
    assert.equal(calls[0].destTargetExistsBeforeCall, false);
    assert.equal(calls[1].destExistsBeforeCall, true);
    assert.equal(calls[1].destTargetExistsBeforeCall, false);
    assert.equal(calls[2].signedPath, dest);
    assert.equal(calls[3].signedPath, destTarget);
    const leftoverTemps = fs
      .readdirSync(destDir)
      .filter((entry) => entry.endsWith(".tmp"));
    assert.deepEqual(leftoverTemps, []);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
