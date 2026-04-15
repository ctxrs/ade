const test = require("node:test");
const assert = require("node:assert/strict");

const {
  TARGET_LABEL,
  TARGET_SPECS,
  parseArgs,
} = require("./codex_crp_bazel.cjs");

test("codex-crp Bazel helper keeps the cross-target platform map explicit", () => {
  assert.equal(TARGET_LABEL, "//core/crates/codex-crp:codex-crp");
  assert.deepEqual(Object.keys(TARGET_SPECS).sort(), [
    "darwin-aarch64",
    "darwin-x86_64",
    "linux-aarch64",
    "linux-x86_64",
  ]);
  assert.equal(TARGET_SPECS["darwin-aarch64"].platformLabel, "//tools/bazel/platforms:darwin_arm64");
  assert.equal(TARGET_SPECS["linux-x86_64"].rustTarget, "x86_64-unknown-linux-gnu");
});

test("codex-crp Bazel helper requires an explicit supported target key", () => {
  assert.deepEqual(parseArgs(["--target-key", "darwin-aarch64"]), { targetKey: "darwin-aarch64" });
  assert.throws(() => parseArgs([]), /--target-key is required/);
  assert.throws(() => parseArgs(["--target-key", "windows-x86_64"]), /unsupported codex-crp Bazel target/);
});
