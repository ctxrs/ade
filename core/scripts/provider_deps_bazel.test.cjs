const test = require("node:test");
const assert = require("node:assert/strict");

const {
  PROVIDER_SPECS,
  TARGET_SPECS,
  parseArgs,
} = require("./provider_deps_bazel.cjs");

test("provider-deps Bazel helper keeps the explicit provider and cross-target map", () => {
  assert.deepEqual(Object.keys(PROVIDER_SPECS).sort(), [
    "acp-crp-bridge",
    "amp",
    "claude-crp",
    "codex",
    "droid",
    "goose",
    "openhands",
    "pi",
  ]);
  assert.equal(PROVIDER_SPECS["acp-crp-bridge"].targetLabel, "//external-harnesses/acp-crp-bridge:acp-crp-bridge");
  assert.equal(PROVIDER_SPECS.amp.artifactKind, "archive");
  assert.equal(PROVIDER_SPECS.amp.targetLabel, "//harness-adapters/example-acp:provider-stage-archive");
  assert.equal(PROVIDER_SPECS["claude-crp"].artifactKind, "archive");
  assert.equal(PROVIDER_SPECS["claude-crp"].targetLabel, "//external-harnesses/claude-crp:provider-stage-archive");
  assert.equal(PROVIDER_SPECS["claude-crp"].passTargetKeyToRun, true);
  assert.equal(PROVIDER_SPECS.codex.artifactKind, "binary");
  assert.equal(PROVIDER_SPECS.codex.resolver, "codex-crp-bazel");
  assert.equal(PROVIDER_SPECS.droid.binaryName, "droid-acp");
  assert.equal(PROVIDER_SPECS.goose.artifactKind, "archive");
  assert.equal(PROVIDER_SPECS.goose.targetLabel, "//core/crates/ctx-provider-accounts:goose-provider-stage-archive");
  assert.equal(PROVIDER_SPECS.goose.passTargetKeyToRun, true);
  assert.equal(PROVIDER_SPECS.openhands.artifactKind, "archive");
  assert.equal(
    PROVIDER_SPECS.openhands.targetLabel,
    "//core/crates/ctx-provider-accounts:openhands-provider-stage-archive",
  );
  assert.equal(PROVIDER_SPECS.openhands.passTargetKeyToRun, true);
  assert.equal(PROVIDER_SPECS.pi.artifactKind, "archive");
  assert.equal(PROVIDER_SPECS.pi.targetLabel, "//harness-adapters/pi-acp:provider-stage-archive");
  assert.deepEqual(Object.keys(TARGET_SPECS).sort(), [
    "darwin-aarch64",
    "darwin-x86_64",
    "linux-aarch64",
    "linux-x86_64",
  ]);
  assert.equal(TARGET_SPECS["darwin-x86_64"].platformLabel, "//tools/bazel/platforms:darwin_x86_64");
});

test("provider-deps Bazel helper requires an explicit supported provider and target", () => {
  assert.deepEqual(parseArgs(["--provider-id", "droid", "--target-key", "linux-x86_64"]), {
    providerId: "droid",
    targetKey: "linux-x86_64",
  });
  assert.deepEqual(parseArgs(["--provider-id", "goose", "--target-key", "darwin-aarch64"]), {
    providerId: "goose",
    targetKey: "darwin-aarch64",
  });
  assert.throws(() => parseArgs([]), /--provider-id is required/);
  assert.throws(
    () => parseArgs(["--provider-id", "not-a-provider", "--target-key", "linux-x86_64"]),
    /unsupported provider-deps Bazel provider/,
  );
  assert.throws(() => parseArgs(["--provider-id", "droid"]), /--target-key is required/);
  assert.throws(() => parseArgs(["--provider-id", "droid", "--target-key", "windows-x86_64"]), /unsupported provider-deps Bazel target/);
});
