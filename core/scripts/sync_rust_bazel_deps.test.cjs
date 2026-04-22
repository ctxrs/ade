const assert = require("node:assert/strict");
const test = require("node:test");

const { parseArgs } = require("./sync_rust_bazel_deps.cjs");

test("parseArgs handles default write mode", () => {
  assert.deepEqual(parseArgs([]), {
    check: false,
    outPath: "",
  });
});

test("parseArgs handles check mode and explicit output path", () => {
  assert.deepEqual(parseArgs(["--check", "--out", "/tmp/generated.bzl"]), {
    check: true,
    outPath: "/tmp/generated.bzl",
  });
});

test("parseArgs rejects missing or unsupported args", () => {
  assert.throws(() => parseArgs(["--out"]), /--out requires a path/);
  assert.throws(() => parseArgs(["--unknown"]), /unsupported arg: --unknown/);
});
