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
