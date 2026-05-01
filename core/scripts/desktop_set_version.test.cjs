const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const { setDesktopVersion } = require("./desktop_set_version.cjs");

const makeCoreRoot = () => fs.mkdtempSync(path.join(os.tmpdir(), "desktop-set-version-test-"));

const writeFixture = (coreRoot) => {
  const files = new Map([
    [
      "apps/desktop/package.json",
      `${JSON.stringify({ name: "desktop", version: "0.1.0" }, null, 2)}\n`,
    ],
    [
      "apps/desktop/src-tauri/tauri.conf.json",
      `${JSON.stringify({ version: "0.1.0" }, null, 2)}\n`,
    ],
    ["apps/desktop/src-tauri/Cargo.toml", '[package]\nname = "ctx-desktop"\nversion = "0.1.0"\n'],
    ["crates/ctx-http/Cargo.toml", '[package]\nname = "ctx-http"\nversion = "0.1.0"\n'],
    [
      "crates/ctx-http/BUILD.bazel",
      'CTX_HTTP_RUSTC_ENV = {\n    "CARGO_PKG_VERSION": "0.1.0",\n}\n',
    ],
  ]);

  for (const [relativePath, contents] of files) {
    const filePath = path.join(coreRoot, relativePath);
    fs.mkdirSync(path.dirname(filePath), { recursive: true });
    fs.writeFileSync(filePath, contents, "utf8");
  }
};

test("setDesktopVersion updates Cargo and Bazel daemon versions", () => {
  const coreRoot = makeCoreRoot();
  writeFixture(coreRoot);

  setDesktopVersion("0.62.42", { root: coreRoot });

  assert.equal(
    JSON.parse(fs.readFileSync(path.join(coreRoot, "apps/desktop/package.json"), "utf8")).version,
    "0.62.42",
  );
  assert.match(
    fs.readFileSync(path.join(coreRoot, "apps/desktop/src-tauri/Cargo.toml"), "utf8"),
    /version = "0\.62\.42"/,
  );
  assert.match(
    fs.readFileSync(path.join(coreRoot, "crates/ctx-http/Cargo.toml"), "utf8"),
    /version = "0\.62\.42"/,
  );
  assert.match(
    fs.readFileSync(path.join(coreRoot, "crates/ctx-http/BUILD.bazel"), "utf8"),
    /"CARGO_PKG_VERSION": "0\.62\.42"/,
  );
});

test("setDesktopVersion is idempotent when Cargo and Bazel versions already match", () => {
  const coreRoot = makeCoreRoot();
  writeFixture(coreRoot);

  setDesktopVersion("0.62.42", { root: coreRoot });

  assert.doesNotThrow(() => {
    setDesktopVersion("0.62.42", { root: coreRoot });
  });
});
