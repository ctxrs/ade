const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");

const coreRoot = path.resolve(__dirname, "..");
const iconPath = path.join(coreRoot, "apps", "desktop", "src-tauri", "icons", "icon.icns");

const requiredIconReps = [
  "icon_16x16.png",
  "icon_16x16@2x.png",
  "icon_32x32.png",
  "icon_32x32@2x.png",
  "icon_128x128.png",
  "icon_128x128@2x.png",
  "icon_256x256.png",
  "icon_256x256@2x.png",
  "icon_512x512.png",
  "icon_512x512@2x.png",
];

test(
  "desktop icon.icns contains all required macOS icon representations",
  { skip: process.platform !== "darwin" ? "requires macOS iconutil" : false },
  () => {
    assert.ok(fs.existsSync(iconPath), `missing icon file at ${iconPath}`);

    const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-icon-reps-"));
    const iconsetOut = path.join(tmpRoot, "icon.iconset");

    try {
      const res = spawnSync("iconutil", ["-c", "iconset", iconPath, "-o", iconsetOut], {
        cwd: coreRoot,
        encoding: "utf8",
      });
      assert.equal(res.status, 0, `iconutil failed: ${res.stderr || res.stdout}`);

      const reps = new Set(fs.readdirSync(iconsetOut));
      for (const requiredRep of requiredIconReps) {
        assert.ok(reps.has(requiredRep), `missing required icon rep: ${requiredRep}`);
      }
    } finally {
      fs.rmSync(tmpRoot, { recursive: true, force: true });
    }
  },
);
