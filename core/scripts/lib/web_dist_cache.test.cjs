const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  ARTIFACT_MARKER,
  WEB_DIST_SYNC_TARGET,
  computeWebDistCacheKey,
  ensureWebDistArtifact,
  resolveDesktopWebDistSource,
  resolveWebDistArtifactDir,
  runWebDistBuild,
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

test("ensureWebDistArtifact materializes the Bazel-run dist once and reuses it on cache hit", () => {
  const coreRoot = createFixtureCoreRoot();
  const volatileRoot = path.join(coreRoot, ".volatile");
  const calls = [];

  const first = ensureWebDistArtifact({
    coreRoot,
    env: { CTX_VOLATILE_ROOT: volatileRoot },
    variant: "desktop-release",
    appVersion: "1.2.3",
    runWebDistBuildImpl: ({ env, destinationDir }) => {
      calls.push({ type: "run", env });
      assert.equal(destinationDir.endsWith(path.join("dist")), true);
      fs.mkdirSync(destinationDir, { recursive: true });
      fs.writeFileSync(path.join(destinationDir, "index.html"), "<html>cached</html>\n");
      return destinationDir;
    },
  });
  const second = ensureWebDistArtifact({
    coreRoot,
    env: { CTX_VOLATILE_ROOT: volatileRoot },
    variant: "desktop-release",
    appVersion: "1.2.3",
    runWebDistBuildImpl: () => {
      throw new Error("cache hit should not rebuild");
    },
  });

  assert.equal(first.reused, false);
  assert.equal(second.reused, true);
  assert.equal(first.distDir, second.distDir);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].type, "run");
  assert.equal(fs.existsSync(path.join(path.dirname(first.distDir), ARTIFACT_MARKER)), true);
});

test("ensureWebDistArtifact fails when the Bazel-run copy does not materialize a dist directory", () => {
  const coreRoot = createFixtureCoreRoot();
  assert.throws(
    () => ensureWebDistArtifact({
      coreRoot,
      runWebDistBuildImpl: () => path.join(coreRoot, "apps", "web", "dist"),
    }),
    /did not materialize/,
  );
});

test("computeWebDistCacheKey changes when relevant web inputs change", () => {
  const coreRoot = createFixtureCoreRoot();
  const before = computeWebDistCacheKey({ coreRoot, variant: "e2e" });
  writeFile(path.join(coreRoot, "apps", "web", "src", "main.tsx"), "console.log('changed');\n");
  const after = computeWebDistCacheKey({ coreRoot, variant: "e2e" });
  assert.notEqual(before, after);
});

test("runWebDistBuild uses bazel run and resolves the workspace dist directory", () => {
  const coreRoot = createFixtureCoreRoot();
  const expectedDist = path.join(coreRoot, "apps", "web", "dist");
  fs.mkdirSync(expectedDist, { recursive: true });
  const spawnCalls = [];

  const distDir = runWebDistBuild({
    coreRoot,
    env: { BUILDBUDDY_API_KEY: "api-key-123" },
    spawnSyncImpl: (command, args, options) => {
      spawnCalls.push({ command, args, options });
      return { status: 0 };
    },
  });

  assert.equal(distDir, expectedDist);
  assert.equal(spawnCalls.length, 1);
  assert.equal(spawnCalls[0].args[1], "run");
  assert.match(spawnCalls[0].args[2], /^--disk_cache=/);
  assert.match(spawnCalls[0].args[3], /^--repository_cache=/);
  assert.equal(spawnCalls[0].args[4], "--remote_header=x-buildbuddy-api-key=api-key-123");
  assert.equal(spawnCalls[0].args[5], WEB_DIST_SYNC_TARGET);
  assert.equal(spawnCalls[0].args[6], "--");
  assert.equal(spawnCalls[0].args[7], expectedDist);
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
