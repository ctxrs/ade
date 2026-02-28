const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");

const coreRoot = path.resolve(__dirname, "..");
const importScript = path.join(coreRoot, "scripts", "desktop_import_bundles.cjs");

test("desktop_import_bundles preserves runtime lock files when source omits them", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-import-"));
  const sourceDir = path.join(tmpRoot, "source");
  const destDir = path.join(tmpRoot, "dest");
  fs.mkdirSync(sourceDir, { recursive: true });
  fs.mkdirSync(destDir, { recursive: true });

  fs.writeFileSync(path.join(sourceDir, "manifest.json"), "{\n  \"providers\": []\n}\n", "utf8");
  fs.mkdirSync(path.join(sourceDir, "providers"), { recursive: true });
  fs.writeFileSync(path.join(sourceDir, "providers", "placeholder.txt"), "provider\n", "utf8");

  const runtimeV1 = "{\n  \"lock\": \"v1\"\n}\n";
  const runtimeV2 = "{\n  \"lock\": \"v2\"\n}\n";
  fs.writeFileSync(path.join(destDir, "runtime_lock.v1.json"), runtimeV1, "utf8");
  fs.writeFileSync(path.join(destDir, "runtime_lock.v2.json"), runtimeV2, "utf8");
  fs.writeFileSync(path.join(destDir, "README.md"), "bundle readme\n", "utf8");
  fs.writeFileSync(path.join(destDir, "lucide-settings.svg"), "<svg/>\n", "utf8");
  fs.writeFileSync(path.join(destDir, ".gitignore"), "*\n", "utf8");
  fs.writeFileSync(path.join(destDir, "stale.txt"), "stale\n", "utf8");

  const res = spawnSync(process.execPath, [importScript, "--from", sourceDir], {
    cwd: coreRoot,
    env: {
      ...process.env,
      CTX_DESKTOP_IMPORT_DEST: destDir,
    },
    encoding: "utf8",
  });
  assert.equal(res.status, 0, `import failed: ${res.stderr || res.stdout}`);

  assert.equal(
    fs.readFileSync(path.join(destDir, "runtime_lock.v1.json"), "utf8"),
    runtimeV1,
  );
  assert.equal(
    fs.readFileSync(path.join(destDir, "runtime_lock.v2.json"), "utf8"),
    runtimeV2,
  );
  assert.ok(fs.existsSync(path.join(destDir, "manifest.json")));
  assert.ok(fs.existsSync(path.join(destDir, "providers", "placeholder.txt")));
  assert.equal(fs.existsSync(path.join(destDir, "stale.txt")), false);

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});

test("desktop_import_bundles normalizes file permissions for bundle readability", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-import-perms-"));
  const sourceDir = path.join(tmpRoot, "source");
  const destDir = path.join(tmpRoot, "dest");
  fs.mkdirSync(path.join(sourceDir, "providers"), { recursive: true });
  fs.writeFileSync(path.join(sourceDir, "manifest.json"), "{\n  \"providers\": []\n}\n", "utf8");
  fs.writeFileSync(path.join(sourceDir, "providers", "script.sh"), "#!/usr/bin/env bash\necho ok\n", "utf8");
  fs.writeFileSync(path.join(sourceDir, "providers", "config.json"), "{}\n", "utf8");
  fs.chmodSync(path.join(sourceDir, "providers", "script.sh"), 0o500);
  fs.chmodSync(path.join(sourceDir, "providers", "config.json"), 0o400);

  const res = spawnSync(process.execPath, [importScript, "--from", sourceDir], {
    cwd: coreRoot,
    env: {
      ...process.env,
      CTX_DESKTOP_IMPORT_DEST: destDir,
    },
    encoding: "utf8",
  });
  assert.equal(res.status, 0, `import failed: ${res.stderr || res.stdout}`);

  const scriptMode = fs.statSync(path.join(destDir, "providers", "script.sh")).mode & 0o777;
  const configMode = fs.statSync(path.join(destDir, "providers", "config.json")).mode & 0o777;
  assert.equal(scriptMode, 0o755);
  assert.equal(configMode, 0o644);

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});
