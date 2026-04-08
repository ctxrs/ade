const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  ARTIFACT_MARKER,
  computeWebDistCacheKey,
  ensureWebDistArtifact,
  resolveDesktopWebDistSource,
  resolveWebDistArtifactDir,
} = require("./web_dist_cache.cjs");

function writeFile(filePath, contents) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, contents);
}

function createFixtureCoreRoot() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-web-dist-cache-"));
  writeFile(path.join(root, "package.json"), JSON.stringify({ private: true }, null, 2));
  writeFile(path.join(root, "pnpm-lock.yaml"), "lockfileVersion: '9.0'\n");
  writeFile(path.join(root, "pnpm-workspace.yaml"), "packages:\n  - apps/*\n  - packages/*\n");
  writeFile(path.join(root, "apps", "web", "package.json"), JSON.stringify({ name: "ctx-web" }, null, 2));
  writeFile(path.join(root, "apps", "web", "index.html"), "<!doctype html><html></html>\n");
  writeFile(path.join(root, "apps", "web", "src", "main.tsx"), "console.log('web');\n");
  writeFile(path.join(root, "packages", "ctx-design", "src", "index.ts"), "export const design = 1;\n");
  return root;
}

test("resolveWebDistArtifactDir uses the volatile artifacts root", () => {
  const coreRoot = createFixtureCoreRoot();
  const volatileRoot = path.join(coreRoot, ".volatile");
  const artifactDir = resolveWebDistArtifactDir({
    coreRoot,
    env: { CTX_VOLATILE_ROOT: volatileRoot },
    variant: "e2e",
  });
  assert.match(artifactDir, new RegExp(`${volatileRoot.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}.+web-dist`));
  assert.match(artifactDir, /dist$/);
});

test("ensureWebDistArtifact builds once and reuses the stable artifact on a cache hit", () => {
  const coreRoot = createFixtureCoreRoot();
  const volatileRoot = path.join(coreRoot, ".volatile");
  const calls = [];
  const fakeViteBin = path.join(coreRoot, "node_modules", ".bin", "vite");
  writeFile(fakeViteBin, "#!/bin/sh\n");
  const spawnSyncImpl = (_command, args) => {
    calls.push(args);
    const outDir = args[2];
    fs.mkdirSync(outDir, { recursive: true });
    fs.writeFileSync(path.join(outDir, "index.html"), "<html>cached</html>\n");
    return { status: 0 };
  };

  const first = ensureWebDistArtifact({
    coreRoot,
    env: { CTX_VOLATILE_ROOT: volatileRoot },
    variant: "desktop-release",
    appVersion: "1.2.3",
    resolveLocalNodeBinImpl: () => fakeViteBin,
    spawnSyncImpl,
  });
  const second = ensureWebDistArtifact({
    coreRoot,
    env: { CTX_VOLATILE_ROOT: volatileRoot },
    variant: "desktop-release",
    appVersion: "1.2.3",
    resolveLocalNodeBinImpl: () => fakeViteBin,
    spawnSyncImpl,
  });

  assert.equal(first.reused, false);
  assert.equal(second.reused, true);
  assert.equal(first.distDir, second.distDir);
  assert.equal(calls.length, 1);
  assert.equal(fs.existsSync(path.join(path.dirname(first.distDir), ARTIFACT_MARKER)), true);
});

test("computeWebDistCacheKey changes when relevant web inputs change", () => {
  const coreRoot = createFixtureCoreRoot();
  const before = computeWebDistCacheKey({ coreRoot, variant: "e2e" });
  writeFile(path.join(coreRoot, "apps", "web", "src", "main.tsx"), "console.log('changed');\n");
  const after = computeWebDistCacheKey({ coreRoot, variant: "e2e" });
  assert.notEqual(before, after);
});

test("resolveDesktopWebDistSource prefers CTX_DESKTOP_WEB_DIST when provided", () => {
  const coreRoot = createFixtureCoreRoot();
  assert.equal(
    resolveDesktopWebDistSource(coreRoot, { CTX_DESKTOP_WEB_DIST: "tmp/web-dist" }),
    path.join(coreRoot, "tmp", "web-dist"),
  );
  assert.equal(
    resolveDesktopWebDistSource(coreRoot, {}),
    path.join(coreRoot, "apps", "web", "dist"),
  );
});
