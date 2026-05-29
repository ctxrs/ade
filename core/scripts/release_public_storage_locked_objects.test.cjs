"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");

const {
  checkLockedStorageObjects,
  collectLockedStorageObjects,
  objectPathFromPublicStorageUrl,
  publicUrlForObjectPath,
} = require("./release_public_storage_locked_objects.cjs");

test("collects release public-storage objects from runtime and Tauri tool locks", () => {
  const entries = collectLockedStorageObjects();
  const paths = entries.map((entry) => entry.objectPath);

  assert.equal(new Set(paths).size, paths.length);
  assert.ok(paths.length >= 14);
  assert.ok(paths.some((objectPath) => objectPath.startsWith("tauri-tools/tauri-apps/binary-releases/releases/download/apprun-old/")));
  assert.ok(paths.some((objectPath) => objectPath.startsWith("tauri-tools/linuxdeploy/linuxdeploy-plugin-appimage/releases/download/continuous/")));
  assert.ok(paths.some((objectPath) => objectPath.startsWith("runtimes/avf-linux-guest/")));
  assert.ok(paths.some((objectPath) => objectPath.startsWith("images/ctx-harness/")));

  for (const entry of entries) {
    assert.equal(entry.url, publicUrlForObjectPath(entry.objectPath));
    assert.equal(objectPathFromPublicStorageUrl(entry.url), entry.objectPath);
  }
});

test("rejects non-ctx public storage origins", () => {
  assert.equal(
    objectPathFromPublicStorageUrl("https://supabase.example.test/storage/v1/object/public/releases/x"),
    "",
  );
  assert.equal(
    objectPathFromPublicStorageUrl("https://api.ctx.rs/functions/v1/download/stable/latest.json"),
    "",
  );
});

test("presence check preserves explicit zero-byte expectations", async () => {
  const runtimeLockPath = require("node:path").join(
    require("node:os").tmpdir(),
    `ctx-runtime-lock-${process.pid}-${Date.now()}.json`,
  );
  const tauriToolsLockPath = require("node:path").join(
    require("node:os").tmpdir(),
    `ctx-tauri-tools-lock-${process.pid}-${Date.now()}.json`,
  );
  require("node:fs").writeFileSync(runtimeLockPath, "{\"schema_version\":2,\"components\":[]}\n", "utf8");
  require("node:fs").writeFileSync(tauriToolsLockPath, JSON.stringify({
    schema_version: 1,
    entries: [{
      asset: "empty.bin",
      id: "empty",
      owner: "owner",
      repo: "repo",
      tag: "tag",
      size_bytes: 0,
    }],
  }), "utf8");

  const result = await checkLockedStorageObjects({
    runtimeLockPath,
    tauriToolsLockPath,
    fetchImpl: async () => ({
      headers: { get: () => "12" },
      ok: true,
      status: 200,
    }),
  });

  assert.deepEqual(result.failures, [
    "tauri-tools/owner/repo/releases/download/tag/empty.bin: size mismatch expected 0 got 12",
  ]);
});
