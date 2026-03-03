const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const { readDesktopVersion } = require("./desktop_version.cjs");

const writeDesktopPackage = (coreRoot, value) => {
  const pkgPath = path.join(coreRoot, "apps", "desktop", "package.json");
  fs.mkdirSync(path.dirname(pkgPath), { recursive: true });
  fs.writeFileSync(pkgPath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
};

const makeCoreRoot = () => fs.mkdtempSync(path.join(os.tmpdir(), "desktop-version-test-"));

test("readDesktopVersion returns apps/desktop package version", () => {
  const coreRoot = makeCoreRoot();
  writeDesktopPackage(coreRoot, { version: "0.4.10" });
  assert.equal(readDesktopVersion(coreRoot), "0.4.10");
});

test("readDesktopVersion throws when version is missing", () => {
  const coreRoot = makeCoreRoot();
  writeDesktopPackage(coreRoot, { version: "  " });
  assert.throws(() => readDesktopVersion(coreRoot), /missing desktop version/);
});
