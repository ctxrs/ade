"use strict";

const assert = require("node:assert/strict");
const childProcess = require("node:child_process");
const path = require("node:path");
const test = require("node:test");

const {
  DEFAULT_LOCK_PATH,
  assertPublishEnv,
  buildUpstreamUrl,
  contentTypeForArchive,
  parseArgs,
  parseRuntimeLock,
  resolveMirrorObject,
} = require("./managed_runtime_mirror.cjs");

test("managed runtime mirror parser covers every locked runtime archive", () => {
  const entries = parseRuntimeLock(DEFAULT_LOCK_PATH);
  assert.equal(entries.length, 18);
  assert.equal(entries.filter((entry) => entry.kind === "node").length, 6);
  assert.equal(entries.filter((entry) => entry.kind === "python" && entry.version === "3.13.13").length, 6);
  assert.equal(entries.filter((entry) => entry.kind === "python" && entry.version === "3.12.13").length, 6);

  for (const entry of entries) {
    assert.match(entry.sha256, /^[0-9a-f]{64}$/);
    assert.equal(entry.bucket, "releases");
    assert.equal(entry.objectPath.startsWith("artifacts/managed-runtimes/"), true);
    assert.equal(entry.mirrorUrl.startsWith("https://api.ctx.rs/storage/v1/object/public/releases/artifacts/managed-runtimes/"), true);
    assert.equal(entry.mirrorUrl.includes("nodejs.org"), false);
    assert.equal(entry.mirrorUrl.includes("github.com"), false);
    assert.equal(entry.mirrorUrl.includes("supabase.co"), false);
  }
});

test("managed runtime publisher derives upstream URLs only for operator-side mirroring", () => {
  const entries = parseRuntimeLock(DEFAULT_LOCK_PATH);
  const node = entries.find((entry) => entry.kind === "node" && entry.target === "linux-x64");
  const python = entries.find((entry) => entry.kind === "python" && entry.version === "3.13.13" && entry.target === "x86_64-unknown-linux-gnu");
  assert.ok(node);
  assert.ok(python);
  assert.equal(
    buildUpstreamUrl(node),
    `https://nodejs.org/dist/v${node.version}/${node.archiveName}`,
  );
  assert.equal(
    buildUpstreamUrl(python),
    `https://github.com/indygreg/python-build-standalone/releases/download/${python.buildTag}/${python.archiveName}`,
  );
});

test("managed runtime mirror object validation rejects non-client mirror origins", () => {
  assert.throws(
    () => resolveMirrorObject("https://api.ctx.rs/functions/v1/download/managed-runtimes/node/24.15.0/node.tar.gz"),
    /public Storage object/,
  );
  assert.throws(
    () => resolveMirrorObject("https://supabase.example.test/storage/v1/object/public/releases/artifacts/managed-runtimes/node.tar.gz"),
    /api\.ctx\.rs/,
  );
  assert.throws(
    () => resolveMirrorObject("https://nodejs.org/dist/v24.15.0/node-v24.15.0-linux-x64.tar.gz"),
    /api\.ctx\.rs/,
  );
});

test("managed runtime mirror CLI supports dry-run publish without credentials", () => {
  const result = childProcess.spawnSync(process.execPath, [
    path.join(__dirname, "managed_runtime_mirror.cjs"),
    "--publish",
    "--dry-run",
  ], {
    cwd: path.resolve(__dirname, ".."),
    encoding: "utf8",
    env: { ...process.env },
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /publish-plan\tnode\tlinux-x64\thttps:\/\/nodejs\.org\//);
  assert.match(result.stdout, /publish-plan\tpython\tx86_64-unknown-linux-gnu\thttps:\/\/github\.com\/indygreg\/python-build-standalone\//);
  assert.match(result.stdout, /https:\/\/api\.ctx\.rs\/storage\/v1\/object\/public\/releases\/artifacts\/managed-runtimes\//);
});

test("managed runtime mirror publish rejects non-ctx public origins before uploading", () => {
  for (const publicOrigin of [
    "http://api.ctx.rs",
    "https://api.ctx.rs:444",
    "https://api.ctx.rs?x=1",
    "https://api.ctx.rs#fragment",
    "https://service:test@api.ctx.rs",
    "https://supabase.example.test",
    "https://api.ctx.rs/storage/v1",
  ]) {
    const result = childProcess.spawnSync(process.execPath, [
      path.join(__dirname, "managed_runtime_mirror.cjs"),
      "--publish",
      "--cache-dir",
      path.join("/tmp", "ctx-managed-runtime-mirror-no-upload"),
    ], {
      cwd: path.resolve(__dirname, ".."),
      encoding: "utf8",
      env: {
        ...process.env,
        RELEASE_PUBLIC_STORAGE_ORIGIN: publicOrigin,
        RELEASE_PUBLIC_STORAGE_BUCKET: "releases",
        RELEASE_R2_ACCESS_KEY_ID: "access",
        RELEASE_R2_ENDPOINT: "https://r2.example.test",
        RELEASE_R2_SECRET_ACCESS_KEY: "secret",
        RELEASE_STORAGE_BUCKET: "ade-releases",
      },
    });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /publishing requires RELEASE_PUBLIC_STORAGE_ORIGIN=https:\/\/api\.ctx\.rs/);
  }
});

test("managed runtime mirror publish accepts the ctx public origin", () => {
  const entries = parseRuntimeLock(DEFAULT_LOCK_PATH);
  for (const publicOrigin of ["https://api.ctx.rs", "https://api.ctx.rs/"]) {
    const { client } = assertPublishEnv({
      RELEASE_PUBLIC_STORAGE_BUCKET: "releases",
      RELEASE_PUBLIC_STORAGE_ORIGIN: publicOrigin,
      RELEASE_R2_ACCESS_KEY_ID: "access",
      RELEASE_R2_ENDPOINT: "https://r2.example.test",
      RELEASE_R2_SECRET_ACCESS_KEY: "secret",
      RELEASE_STORAGE_BUCKET: "ade-releases",
    }, entries);
    assert.equal(client.config.publicOrigin, "https://api.ctx.rs");
    assert.equal(client.config.publicBucket, "releases");
    assert.equal(client.config.bucket, "ade-releases");
  }
});

test("managed runtime mirror CLI defaults to presence checking", () => {
  assert.equal(parseArgs([]).mode, "check-presence");
  assert.equal(contentTypeForArchive("runtime.zip"), "application/zip");
  assert.equal(contentTypeForArchive("runtime.tar.gz"), "application/gzip");
});
