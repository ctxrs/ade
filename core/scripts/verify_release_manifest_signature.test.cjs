const test = require("node:test");
const assert = require("node:assert/strict");
const childProcess = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const verifyScript = path.join(repoRoot, "core", "scripts", "verify_release_manifest_signature.cjs");
const parserScript = path.join(repoRoot, "scripts", "lib", "read_tauri_signature.awk");
const ed25519SpkiPrefix = Buffer.from("302a300506032b6570032100", "hex");

function createTauriUpdaterSignature({ manifestPath, privateKey, keyId }) {
  const manifestBytes = fs.readFileSync(manifestPath);
  const trustedComment = `trusted comment: timestamp:1772585616\tfile:${path.basename(manifestPath)}`;
  const signatureBytes = crypto.sign(
    null,
    crypto.createHash("blake2b512").update(manifestBytes).digest(),
    privateKey,
  );
  const globalSignatureBytes = crypto.sign(
    null,
    Buffer.concat([
      signatureBytes,
      Buffer.from(trustedComment.slice("trusted comment: ".length), "utf8"),
    ]),
    privateKey,
  );
  const signatureText = [
    "untrusted comment: signature from tauri secret key",
    Buffer.concat([Buffer.from([0x45, 0x44]), keyId, signatureBytes]).toString("base64"),
    trustedComment,
    globalSignatureBytes.toString("base64"),
    "",
  ].join("\n");
  return Buffer.from(signatureText, "utf8").toString("base64");
}

test("release manifest verifier accepts compact Tauri updater signatures parsed from signer output", (t) => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "release-manifest-signature-"));
  t.after(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  const manifestPath = path.join(tmpDir, "latest.json");
  const signerLogPath = path.join(tmpDir, "tauri-signer.log");
  const signaturePath = path.join(tmpDir, "latest.json.sig");
  const pubkeyPath = path.join(tmpDir, "updater_pubkey.txt");
  fs.writeFileSync(
    manifestPath,
    `${JSON.stringify({
      channel: "stable",
      latest_version: "0.1.0",
      platforms: {},
      published_at: "2026-04-29T00:00:00Z",
      source_commit: "test-source-commit",
    }, null, 2)}\n`,
    "utf8",
  );

  const { publicKey, privateKey } = crypto.generateKeyPairSync("ed25519");
  const publicDer = publicKey.export({ format: "der", type: "spki" });
  assert.deepEqual(publicDer.subarray(0, ed25519SpkiPrefix.length), ed25519SpkiPrefix);
  const keyId = crypto.randomBytes(8);
  fs.writeFileSync(
    pubkeyPath,
    [
      `untrusted comment: minisign public key: ${keyId.toString("hex")}`,
      Buffer.concat([Buffer.from([0x45, 0x64]), keyId, publicDer.subarray(-32)]).toString("base64"),
      "",
    ].join("\n"),
    "utf8",
  );

  const compactSignature = createTauriUpdaterSignature({ manifestPath, privateKey, keyId });
  fs.writeFileSync(
    signerLogPath,
    `Your file was signed successfully\n\nPublic signature:\n${compactSignature}\n`,
    "utf8",
  );
  const parsedSignature = childProcess.execFileSync("awk", ["-f", parserScript, signerLogPath], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  fs.writeFileSync(signaturePath, parsedSignature, "utf8");

  const output = childProcess.execFileSync(
    process.execPath,
    [verifyScript, manifestPath, signaturePath],
    {
      cwd: repoRoot,
      encoding: "utf8",
      env: {
        ...process.env,
        CTX_RELEASE_MANIFEST_PUBKEY_FILE: pubkeyPath,
      },
    },
  );
  assert.match(output, /release manifest signature: OK/);
});
