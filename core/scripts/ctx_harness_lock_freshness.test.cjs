const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const crypto = require("node:crypto");

const {
  expectedObjectPath,
  validateCtxHarnessLockFreshness,
} = require("./ctx_harness_lock_freshness.cjs");

const sha256File = (filePath) => crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");

const writeImageTar = (filePath, body = "ctx-harness-image\n") => {
  fs.writeFileSync(filePath, body, "utf8");
  return sha256File(filePath);
};

const makeLock = ({ arch, sha256, uri }) => ({
  version: 2,
  components: [
    {
      kind: "image",
      id: "ctx-harness",
      os: "linux",
      arch,
      variant: "default",
      version: "locked",
      sources: [
        { source_type: "local", build: "desktop:prep" },
        { source_type: "ci", uri, sha256 },
      ],
    },
  ],
});

test("validateCtxHarnessLockFreshness accepts matching managed metadata", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-harness-lock-freshness-"));
  const imageTar = path.join(tmpRoot, "ctx-harness-linux-aarch64.tar");
  const sha = writeImageTar(imageTar);
  const lock = makeLock({
    arch: "aarch64",
    sha256: sha,
    uri: `https://example.invalid/${expectedObjectPath("aarch64", sha)}`,
  });

  const result = validateCtxHarnessLockFreshness({ imageTar, lock, arch: "aarch64" });
  assert.equal(result.ok, true, result.errors.join("\n"));

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});

test("validateCtxHarnessLockFreshness reports stale ci sha clearly", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-harness-lock-freshness-drift-"));
  const imageTar = path.join(tmpRoot, "ctx-harness-linux-aarch64.tar");
  writeImageTar(imageTar);
  const lock = makeLock({
    arch: "aarch64",
    sha256: "0".repeat(64),
    uri: `https://example.invalid/${expectedObjectPath("aarch64", "0".repeat(64))}`,
  });

  const result = validateCtxHarnessLockFreshness({ imageTar, lock, arch: "aarch64" });
  assert.equal(result.ok, false);
  assert.ok(result.errors.some((entry) => entry.includes("ci sha drift")));

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});

test("validateCtxHarnessLockFreshness requires the local desktop prep source", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-harness-lock-freshness-local-"));
  const imageTar = path.join(tmpRoot, "ctx-harness-linux-x86_64.tar");
  const sha = writeImageTar(imageTar);
  const lock = {
    version: 2,
    components: [
      {
        kind: "image",
        id: "ctx-harness",
        os: "linux",
        arch: "x86_64",
        variant: "default",
        version: "locked",
        sources: [
          { source_type: "ci", uri: `https://example.invalid/${expectedObjectPath("x86_64", sha)}`, sha256: sha },
        ],
      },
    ],
  };

  const result = validateCtxHarnessLockFreshness({ imageTar, lock, arch: "x86_64" });
  assert.equal(result.ok, false);
  assert.ok(result.errors.some((entry) => entry.includes("desktop:prep source")));

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});
