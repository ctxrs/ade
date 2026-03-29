const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");

test("tauri tools lock has complete pinned entries", async () => {
  const modulePath = path.resolve(__dirname, "..", "..", "scripts", "lib", "tauri_tools_lock.mjs");
  const mod = await import(modulePath);
  const { lock } = mod.loadLock(mod.DEFAULT_LOCK_PATH);
  assert.equal(lock.schema_version, 1);
  assert.ok(Array.isArray(lock.entries));
  assert.ok(lock.entries.length >= 6);

  const seen = new Set();
  for (const entry of lock.entries) {
    mod.ensureEntrySchema(entry);
    assert.equal(typeof entry.id, "string");
    assert.ok(entry.id.length > 0);
    assert.equal(seen.has(entry.id), false, `duplicate lock id: ${entry.id}`);
    seen.add(entry.id);
    assert.match(String(entry.sha256 || ""), /^[a-f0-9]{64}$/i, `invalid sha for ${entry.id}`);
    assert.ok(Number(entry.size_bytes) > 0, `invalid size for ${entry.id}`);
    const rel = mod.mirrorRelativePathFor(entry);
    assert.equal(
      rel,
      `tauri-tools/${entry.owner}/${entry.repo}/releases/download/${entry.tag}/${entry.asset}`,
    );
  }

  assert.ok(seen.has("apprun-x64"));
  assert.ok(seen.has("apprun-arm64"));
  assert.ok(seen.has("linuxdeploy-x64"));
  assert.ok(seen.has("linuxdeploy-arm64"));
  assert.ok(seen.has("linuxdeploy-plugin-appimage-x64"));
  assert.ok(seen.has("linuxdeploy-plugin-appimage-arm64"));
});
