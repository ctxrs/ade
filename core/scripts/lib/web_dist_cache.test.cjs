const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const childProcess = require("node:child_process");

const {
  ARTIFACT_MARKER,
  WEB_DIST_SYNC_TARGET,
  computeWebDistCacheKey,
  ensureWebDistArtifact,
  resolveDirectRunBazelVersion,
  resolveDesktopWebDistSource,
  resolveWebDistArtifactDir,
  runWebDistBuild,
} = require("./web_dist_cache.cjs");
const { HOST_HEAVY_BUDGET_KEY } = require("./host_job_budget.cjs");

function writeFile(filePath, contents) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, contents);
}

function createFixtureCoreRoot() {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-web-dist-cache-"));
  const coreRoot = path.join(repoRoot, "core");
  writeFile(path.join(repoRoot, ".bazelversion"), "buildbuddy-io/5.0.321\n9.0.1\n");
  writeFile(path.join(coreRoot, "package.json"), JSON.stringify({ private: true }, null, 2));
  writeFile(path.join(coreRoot, "pnpm-lock.yaml"), "lockfileVersion: '9.0'\n");
  writeFile(path.join(coreRoot, "pnpm-workspace.yaml"), "packages:\n  - apps/*\n  - packages/*\n");
  writeFile(path.join(coreRoot, "apps", "web", "package.json"), JSON.stringify({ name: "ctx-web" }, null, 2));
  writeFile(path.join(coreRoot, "apps", "web", "index.html"), "<!doctype html><html></html>\n");
  writeFile(path.join(coreRoot, "apps", "web", "src", "main.tsx"), "console.log('web');\n");
  writeFile(path.join(coreRoot, "packages", "ctx-design", "src", "index.ts"), "export const design = 1;\n");
  return coreRoot;
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
  const budgetCalls = [];

  const distDir = runWebDistBuild({
    coreRoot,
    env: { BUILDBUDDY_API_KEY: "api-key-123" },
    spawnSyncImpl: (command, args, options) => {
      spawnCalls.push({ command, args, options });
      return { status: 0 };
    },
    withHostJobBudgetImpl: (options, fn) => {
      budgetCalls.push(options);
      return fn();
    },
  });

  assert.equal(distDir, expectedDist);
  assert.equal(budgetCalls.length, 1);
  assert.equal(budgetCalls[0].budgetKey, HOST_HEAVY_BUDGET_KEY);
  assert.equal(spawnCalls.length, 1);
  assert.equal(spawnCalls[0].args[1], "run");
  assert.match(spawnCalls[0].args[2], /^--disk_cache=/);
  assert.match(spawnCalls[0].args[3], /^--repository_cache=/);
  assert.equal(spawnCalls[0].args[4], "--remote_header=x-buildbuddy-api-key=api-key-123");
  assert.equal(spawnCalls[0].args[5], WEB_DIST_SYNC_TARGET);
  assert.equal(spawnCalls[0].args[6], "--");
  assert.equal(spawnCalls[0].args[7], expectedDist);
  assert.equal(spawnCalls[0].options.env.USE_BAZEL_VERSION, "9.0.1");
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

test("dist_sync_tool builds from a minimal temp workspace without writing dist into the real checkout", () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-web-dist-sync-"));
  const coreRoot = path.join(repoRoot, "core");
  const webRoot = path.join(coreRoot, "apps", "web");
  const outputDir = path.join(repoRoot, "out", "dist");
  const tmpRoot = path.join(repoRoot, "tmp");
  const viteBin = path.join(webRoot, "node_modules", ".bin", "vite");
  const scriptPath = path.resolve(__dirname, "..", "..", "apps", "web", "dist_sync_tool.sh");

  writeFile(path.join(coreRoot, "package.json"), JSON.stringify({ private: true }, null, 2));
  writeFile(path.join(coreRoot, "pnpm-lock.yaml"), "lockfileVersion: '9.0'\n");
  writeFile(path.join(coreRoot, "pnpm-workspace.yaml"), "packages:\n  - apps/*\n  - packages/*\n");
  writeFile(path.join(webRoot, "package.json"), JSON.stringify({ name: "ctx-web" }, null, 2));
  writeFile(path.join(webRoot, "vite.config.ts"), "export default {};\n");
  writeFile(path.join(webRoot, "tsconfig.json"), JSON.stringify({ compilerOptions: {} }, null, 2));
  writeFile(path.join(webRoot, "index.html"), "<!doctype html><html></html>\n");
  writeFile(path.join(webRoot, "postcss.config.cjs"), "module.exports = {};\n");
  writeFile(path.join(webRoot, "tailwind.config.cjs"), "module.exports = { content: [] };\n");
  writeFile(path.join(webRoot, "src", "main.tsx"), "console.log('web');\n");
  writeFile(path.join(webRoot, "public", "favicon.ico"), "icon\n");
  writeFile(path.join(coreRoot, "packages", "ctx-design", "src", "index.ts"), "export const design = 1;\n");
  writeFile(path.join(coreRoot, "node_modules", ".keep"), "");
  fs.mkdirSync(tmpRoot, { recursive: true });
  fs.mkdirSync(path.dirname(viteBin), { recursive: true });
  fs.writeFileSync(
    viteBin,
    [
      "#!/usr/bin/env bash",
      "set -euo pipefail",
      "test -f package.json",
      "test -f ../../package.json",
      "test -L src",
      "test -L public",
      "test -L ../../packages",
      "test -d ../../node_modules",
      "test \"$PWD\" != \"$BUILD_WORKSPACE_DIRECTORY/core/apps/web\"",
      "mkdir -p dist",
      "printf '<html>ok</html>\\n' > dist/index.html",
    ].join("\n"),
    { mode: 0o755 },
  );

  const result = childProcess.spawnSync(scriptPath, [outputDir], {
    env: {
      ...process.env,
      BUILD_WORKSPACE_DIRECTORY: repoRoot,
      RUNFILES_DIR: "",
      TEST_WORKSPACE: "",
      TMPDIR: tmpRoot,
    },
    encoding: "utf8",
  });

  assert.equal(result.status, 0, result.stderr || result.stdout);
  assert.equal(fs.existsSync(path.join(outputDir, "index.html")), true);
  assert.equal(fs.existsSync(path.join(webRoot, "dist")), false);
});

test("resolveDirectRunBazelVersion uses the second .bazelversion line for buildbuddy wrappers", () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-bazelversion-"));
  writeFile(path.join(repoRoot, ".bazelversion"), "buildbuddy-io/5.0.321\n9.0.1\n");
  assert.equal(resolveDirectRunBazelVersion({ repoRoot }), "9.0.1");
});

test("resolveDirectRunBazelVersion preserves an explicit USE_BAZEL_VERSION override", () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-bazelversion-explicit-"));
  writeFile(path.join(repoRoot, ".bazelversion"), "buildbuddy-io/5.0.321\n9.0.1\n");
  assert.equal(
    resolveDirectRunBazelVersion({
      repoRoot,
      env: { USE_BAZEL_VERSION: "8.2.0" },
    }),
    "8.2.0",
  );
});

test("resolveDirectRunBazelVersion ignores plain Bazel .bazelversion pins", () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-bazelversion-plain-"));
  writeFile(path.join(repoRoot, ".bazelversion"), "9.0.1\n");
  assert.equal(resolveDirectRunBazelVersion({ repoRoot }), "");
});
