const assert = require("node:assert/strict");
const childProcess = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const scriptPath = path.join(__dirname, "verify_bundle_manifest_closure.cjs");

function sha256(contents) {
  return crypto.createHash("sha256").update(contents).digest("hex");
}

function writeExecutable(filePath, contents) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, contents);
  fs.chmodSync(filePath, 0o755);
}

function runVerifier(bundleDir) {
  return childProcess.spawnSync("node", [scriptPath, bundleDir], {
    encoding: "utf8",
  });
}

test("bundle manifest closure verifies declared files and hashes", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-bundle-closure-ok-"));
  try {
    const providerPayload = Buffer.from("#!/bin/sh\nexit 0\n");
    const runtimePayload = Buffer.from("#!/bin/sh\nexit 0\n");
    const imagePayload = Buffer.from("image tar\n");
    const daemonPayload = Buffer.from("#!/bin/sh\nexit 0\n");
    writeExecutable(path.join(root, "providers", "codex", "bin", "codex"), providerPayload);
    writeExecutable(path.join(root, "runtimes", "ctx-mcp", "linux", "x86_64", "0.1.0", "ctx-mcp"), runtimePayload);
    fs.mkdirSync(path.join(root, "images"), { recursive: true });
    fs.writeFileSync(path.join(root, "images", "ctx-harness.tar"), imagePayload);
    writeExecutable(path.join(root, "daemons", "ctx-linux-x64"), daemonPayload);
    fs.writeFileSync(
      path.join(root, "manifest.json"),
      `${JSON.stringify(
        {
          version: 1,
          providers: [
            {
              id: "codex",
              protocol: "crp",
              version: "1.0.0",
              os: "linux",
              arch: "x86_64",
              sha256: sha256(providerPayload),
              command: "providers/codex/bin/codex",
            },
          ],
          runtimes: [
            {
              id: "ctx-mcp",
              version: "0.1.0",
              os: "linux",
              arch: "x86_64",
              sha256: sha256(runtimePayload),
              root: "runtimes/ctx-mcp/linux/x86_64/0.1.0",
              bin: "ctx-mcp",
            },
          ],
          images: [
            {
              id: "ctx-harness",
              version: "1.0.0",
              os: "linux",
              arch: "x86_64",
              sha256: sha256(imagePayload),
              tar: "images/ctx-harness.tar",
              image: "ctx-harness:fixture",
            },
          ],
          daemons: [
            {
              id: "ctx-daemon",
              os: "linux",
              arch: "x86_64",
              sha256: sha256(daemonPayload),
              bin: "daemons/ctx-linux-x64",
            },
          ],
        },
        null,
        2,
      )}\n`,
    );

    const result = runVerifier(root);
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
    assert.match(result.stdout, /providers=1 runtimes=1 images=1 daemons=1/);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("bundle manifest closure fails when a declared runtime binary is missing", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-bundle-closure-missing-runtime-"));
  try {
    fs.writeFileSync(
      path.join(root, "manifest.json"),
      `${JSON.stringify(
        {
          version: 1,
          providers: [],
          runtimes: [
            {
              id: "ctx-mcp",
              version: "0.1.0",
              os: "linux",
              arch: "x86_64",
              sha256: "0".repeat(64),
              root: "runtimes/ctx-mcp/linux/x86_64/0.1.0",
              bin: "ctx-mcp",
            },
          ],
          images: [],
          daemons: [],
        },
        null,
        2,
      )}\n`,
    );

    const result = runVerifier(root);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /manifest\.runtimes\[0\]\.root is missing/);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
