const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("fs");
const os = require("os");
const path = require("path");

const {
  normalizePermissionsRecursive,
  parseArgs,
} = require("./desktop_normalize_bundle_permissions.cjs");

test("parseArgs requires --dir", () => {
  assert.throws(() => parseArgs([]), /missing required --dir/);
  assert.deepEqual(parseArgs(["--dir", "bundles"]), { dir: "bundles" });
});

test("normalizePermissionsRecursive normalizes file and directory permissions", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-bundle-perms-"));
  const nestedDir = path.join(tmpRoot, "providers", "demo");
  fs.mkdirSync(nestedDir, { recursive: true });
  const execFile = path.join(nestedDir, "runner.sh");
  const dataFile = path.join(nestedDir, "meta.json");
  fs.writeFileSync(execFile, "#!/usr/bin/env bash\necho ok\n", "utf8");
  fs.writeFileSync(dataFile, "{}\n", "utf8");
  fs.chmodSync(path.join(tmpRoot, "providers"), 0o700);
  fs.chmodSync(nestedDir, 0o700);
  fs.chmodSync(execFile, 0o500);
  fs.chmodSync(dataFile, 0o400);

  normalizePermissionsRecursive(tmpRoot);

  assert.equal(fs.statSync(path.join(tmpRoot, "providers")).mode & 0o777, 0o755);
  assert.equal(fs.statSync(nestedDir).mode & 0o777, 0o755);
  assert.equal(fs.statSync(execFile).mode & 0o777, 0o755);
  assert.equal(fs.statSync(dataFile).mode & 0o777, 0o644);

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});
