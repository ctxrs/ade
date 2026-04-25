import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  copyRuntimeTree,
  materializePlaywrightBrowsers,
  parseArgs,
} from "./materialize-playwright-browsers.mjs";

const createSpy = (impl = () => undefined) => {
  const calls = [];
  const spy = (...args) => {
    calls.push(args);
    return impl(...args);
  };
  spy.calls = calls;
  return spy;
};

const writeFile = (root, relativePath, contents = "", mode = 0o644) => {
  const absolutePath = path.join(root, relativePath);
  fs.mkdirSync(path.dirname(absolutePath), { recursive: true });
  fs.writeFileSync(absolutePath, contents);
  fs.chmodSync(absolutePath, mode);
  return absolutePath;
};

const writeSymlink = (root, relativePath, target) => {
  const absolutePath = path.join(root, relativePath);
  fs.mkdirSync(path.dirname(absolutePath), { recursive: true });
  fs.symlinkSync(target, absolutePath);
  return absolutePath;
};

test("materialize-playwright-browsers parses required browser materialization args", () => {
  assert.deepEqual(parseArgs([
    "--out-dir",
    "playwright-browsers",
    "--runtime-manifest",
    "playwright-runtime-manifest.json",
    "--browser",
    "firefox",
    "--browser",
    "webkit",
    "--browser",
    "chromium",
  ]), {
    browsers: ["firefox", "webkit", "chromium"],
    outDir: "playwright-browsers",
    runtimeManifest: "playwright-runtime-manifest.json",
  });
});

test("materialize-playwright-browsers rejects missing or unsupported args", () => {
  assert.throws(() => parseArgs([]), /missing --out-dir/u);
  assert.throws(() => parseArgs([
    "--out-dir",
    "playwright-browsers",
    "--runtime-manifest",
    "playwright-runtime-manifest.json",
    "--browser",
    "safari",
  ]), /unsupported Playwright browser/u);
});

test("copyRuntimeTree follows symlinks and preserves executable bits", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-copy-runtime-tree-"));
  const sourceDir = path.join(root, "source");
  const targetDir = path.join(root, "target");
  writeFile(sourceDir, "bin/chrome", "#!/bin/sh\n", 0o755);
  writeSymlink(sourceDir, "chrome", "bin/chrome");
  writeSymlink(sourceDir, "current-bin", "bin");

  copyRuntimeTree(sourceDir, targetDir);

  const copiedBinary = path.join(targetDir, "bin", "chrome");
  const copiedAlias = path.join(targetDir, "chrome");
  assert.equal(fs.readFileSync(copiedBinary, "utf8"), "#!/bin/sh\n");
  assert.equal(fs.statSync(copiedBinary).mode & 0o777, 0o755);
  assert.equal(fs.readFileSync(copiedAlias, "utf8"), "#!/bin/sh\n");
  assert.equal(fs.statSync(copiedAlias).mode & 0o777, 0o755);
  assert.equal(fs.lstatSync(copiedAlias).isSymbolicLink(), false);
  assert.equal(fs.statSync(path.join(targetDir, "current-bin")).isDirectory(), true);
  assert.equal(fs.readFileSync(path.join(targetDir, "current-bin", "chrome"), "utf8"), "#!/bin/sh\n");
});

test("materialize-playwright-browsers materializes the requested browsers for every locked platform", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-materialize-playwright-"));
  const workspaceRoot = path.join(root, "workspace");
  const packageWorkDir = path.join(workspaceRoot, "core", "apps", "web");
  const outDir = path.join(packageWorkDir, "playwright-browsers");
  const runtimeRepoDir = path.join(workspaceRoot, "external", "playwright_browser_runtime");
  const manifestPath = path.join(runtimeRepoDir, "runtime_manifest.json");
  fs.mkdirSync(packageWorkDir, { recursive: true });
  writeFile(runtimeRepoDir, "runtime_trees/mac15-arm64/firefox-1497/firefox-bin", "", 0o755);
  writeFile(runtimeRepoDir, "runtime_trees/mac15-arm64/webkit-2227/MiniBrowser", "", 0o755);
  writeSymlink(runtimeRepoDir, "runtime_trees/mac15-arm64/webkit-2227/CurrentMiniBrowser", "MiniBrowser");
  writeFile(runtimeRepoDir, "runtime_trees/mac15-arm64/chromium-1200/chrome", "", 0o755);
  writeFile(runtimeRepoDir, "runtime_trees/ubuntu22.04-x64/firefox-1497/firefox-bin", "", 0o755);
  writeFile(runtimeRepoDir, "runtime_trees/ubuntu22.04-x64/webkit-2227/MiniBrowser", "", 0o755);
  writeFile(runtimeRepoDir, "runtime_trees/ubuntu22.04-x64/chromium-1200/chrome", "", 0o755);
  fs.writeFileSync(manifestPath, JSON.stringify({
    platforms: {
      "mac15-arm64": {
        chromium: { directory: "chromium-1200", path: "runtime_trees/mac15-arm64/chromium-1200" },
        firefox: { directory: "firefox-1497", path: "runtime_trees/mac15-arm64/firefox-1497" },
        webkit: { directory: "webkit-2227", path: "runtime_trees/mac15-arm64/webkit-2227" },
      },
      "ubuntu22.04-x64": {
        chromium: { directory: "chromium-1200", path: "runtime_trees/ubuntu22.04-x64/chromium-1200" },
        firefox: { directory: "firefox-1497", path: "runtime_trees/ubuntu22.04-x64/firefox-1497" },
        webkit: { directory: "webkit-2227", path: "runtime_trees/ubuntu22.04-x64/webkit-2227" },
      },
    },
  }));

  const cwd = process.cwd();
  process.chdir(packageWorkDir);
  try {
    const materializedOutDir = materializePlaywrightBrowsers({
      browsers: ["firefox", "webkit", "chromium"],
      outDir: "playwright-browsers",
      runtimeManifest: manifestPath,
    });
    assert.equal(fs.realpathSync(materializedOutDir), fs.realpathSync(outDir));
  } finally {
    process.chdir(cwd);
  }

  assert.equal(fs.statSync(outDir).isDirectory(), true);
  assert.equal(fs.existsSync(path.join(outDir, "mac15-arm64", "firefox-1497", "firefox-bin")), true);
  assert.equal(fs.statSync(path.join(outDir, "mac15-arm64", "firefox-1497", "firefox-bin")).mode & 0o777, 0o755);
  assert.equal(fs.existsSync(path.join(outDir, "mac15-arm64", "webkit-2227", "MiniBrowser")), true);
  assert.equal(
    fs.lstatSync(path.join(outDir, "mac15-arm64", "webkit-2227", "CurrentMiniBrowser")).isSymbolicLink(),
    false,
  );
  assert.equal(fs.existsSync(path.join(outDir, "mac15-arm64", "chromium-1200", "chrome")), true);
  assert.equal(fs.existsSync(path.join(outDir, "ubuntu22.04-x64", "chromium-1200", "INSTALLATION_COMPLETE")), true);
});

test("materialize-playwright-browsers resolves the runtime manifest from Bazel runfiles", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-materialize-playwright-runfiles-"));
  const packageWorkDir = path.join(root, "execroot", "core", "apps", "web");
  const runfilesDir = path.join(root, "runfiles");
  const runtimeRepoDir = path.join(runfilesDir, "_main", "external", "playwright_browser_runtime");
  const manifestPath = path.join(runtimeRepoDir, "runtime_manifest.json");
  fs.mkdirSync(packageWorkDir, { recursive: true });
  writeFile(runtimeRepoDir, "runtime_trees/mac15-arm64/chromium-1200/chrome", "", 0o755);
  fs.writeFileSync(manifestPath, JSON.stringify({
    platforms: {
      "mac15-arm64": {
        chromium: { directory: "chromium-1200", path: "runtime_trees/mac15-arm64/chromium-1200" },
      },
    },
  }));

  const cwd = process.cwd();
  process.chdir(packageWorkDir);
  try {
    const materializedOutDir = materializePlaywrightBrowsers({
      browsers: ["chromium"],
      outDir: "playwright-browsers",
      runtimeManifest: "external/playwright_browser_runtime/runtime_manifest.json",
    }, {
      env: {
        RUNFILES_DIR: runfilesDir,
      },
    });
    assert.equal(
      fs.realpathSync(materializedOutDir),
      fs.realpathSync(path.join(packageWorkDir, "playwright-browsers")),
    );
  } finally {
    process.chdir(cwd);
  }

  assert.equal(
    fs.existsSync(path.join(packageWorkDir, "playwright-browsers", "mac15-arm64", "chromium-1200", "chrome")),
    true,
  );
});

test("materialize-playwright-browsers fails closed under Bazel metadata instead of walking up into the checkout", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-materialize-playwright-failclosed-"));
  const workspaceRoot = path.join(root, "workspace");
  const packageWorkDir = path.join(workspaceRoot, "core", "apps", "web");
  const runtimeRepoDir = path.join(workspaceRoot, "external", "playwright_browser_runtime");
  const manifestPath = path.join(runtimeRepoDir, "runtime_manifest.json");
  const runfilesDir = path.join(root, "empty-runfiles");
  fs.mkdirSync(packageWorkDir, { recursive: true });
  fs.mkdirSync(runtimeRepoDir, { recursive: true });
  fs.mkdirSync(runfilesDir, { recursive: true });
  fs.writeFileSync(manifestPath, JSON.stringify({
    platforms: {
      "mac15-arm64": {
        webkit: { directory: "webkit-2227", path: "runtime_trees/mac15-arm64/webkit-2227" },
      },
    },
  }));

  const cwd = process.cwd();
  process.chdir(packageWorkDir);
  try {
    assert.throws(() => materializePlaywrightBrowsers({
      browsers: ["webkit"],
      outDir: "playwright-browsers",
      runtimeManifest: "external/playwright_browser_runtime/runtime_manifest.json",
    }, {
      env: {
        RUNFILES_DIR: runfilesDir,
      },
    }), /declared runtime input does not exist/u);
  } finally {
    process.chdir(cwd);
  }
});

test("materialize-playwright-browsers accepts direct sandbox-relative runtime manifest paths under Bazel metadata", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-materialize-playwright-direct-relative-"));
  const packageWorkDir = path.join(root, "core", "apps", "web");
  const runtimeRepoDir = path.join(root, "core", "apps", "playwright-browser-runtime");
  const manifestPath = path.join(runtimeRepoDir, "runtime_manifest.json");
  const runfilesDir = path.join(root, "empty-runfiles");
  fs.mkdirSync(packageWorkDir, { recursive: true });
  fs.mkdirSync(runfilesDir, { recursive: true });
  writeFile(runtimeRepoDir, "runtime_trees/mac15-arm64/firefox-1497/firefox-bin", "", 0o755);
  fs.writeFileSync(manifestPath, JSON.stringify({
    platforms: {
      "mac15-arm64": {
        firefox: { directory: "firefox-1497", path: "runtime_trees/mac15-arm64/firefox-1497" },
      },
    },
  }));

  const cwd = process.cwd();
  process.chdir(packageWorkDir);
  try {
    const materializedOutDir = materializePlaywrightBrowsers({
      browsers: ["firefox"],
      outDir: "playwright-browsers",
      runtimeManifest: "../playwright-browser-runtime/runtime_manifest.json",
    }, {
      env: {
        RUNFILES_DIR: runfilesDir,
      },
    });
    assert.equal(
      fs.realpathSync(materializedOutDir),
      fs.realpathSync(path.join(packageWorkDir, "playwright-browsers")),
    );
  } finally {
    process.chdir(cwd);
  }

  assert.equal(
    fs.existsSync(path.join(packageWorkDir, "playwright-browsers", "mac15-arm64", "firefox-1497", "firefox-bin")),
    true,
  );
});

test("materialize-playwright-browsers accepts Bazel external-repo sandbox paths under Bazel metadata", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-materialize-playwright-external-repo-"));
  const packageWorkDir = path.join(root, "_main", "core", "apps", "web");
  const runtimeRepoDir = path.join(
    root,
    "_main",
    "external",
    "+playwright_browser_runtime_repository+playwright_browser_runtime_mac15_arm64",
  );
  const manifestPath = path.join(runtimeRepoDir, "runtime_manifest.json");
  const runfilesDir = path.join(root, "empty-runfiles");
  fs.mkdirSync(packageWorkDir, { recursive: true });
  fs.mkdirSync(runfilesDir, { recursive: true });
  writeFile(runtimeRepoDir, "runtime_trees/mac15-arm64/webkit-2227/MiniBrowser", "", 0o755);
  fs.writeFileSync(manifestPath, JSON.stringify({
    platforms: {
      "mac15-arm64": {
        webkit: { directory: "webkit-2227", path: "runtime_trees/mac15-arm64/webkit-2227" },
      },
    },
  }));

  const cwd = process.cwd();
  process.chdir(packageWorkDir);
  try {
    const materializedOutDir = materializePlaywrightBrowsers({
      browsers: ["webkit"],
      outDir: "playwright-browsers",
      runtimeManifest: "external/+playwright_browser_runtime_repository+playwright_browser_runtime_mac15_arm64/runtime_manifest.json",
    }, {
      env: {
        RUNFILES_DIR: runfilesDir,
      },
    });
    assert.equal(
      fs.realpathSync(materializedOutDir),
      fs.realpathSync(path.join(packageWorkDir, "playwright-browsers")),
    );
  } finally {
    process.chdir(cwd);
  }

  assert.equal(
    fs.existsSync(path.join(packageWorkDir, "playwright-browsers", "mac15-arm64", "webkit-2227", "MiniBrowser")),
    true,
  );
});

test("materialize-playwright-browsers fails when a platform manifest does not describe a requested browser", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-materialize-playwright-missing-browser-"));
  const packageWorkDir = path.join(root, "core", "apps", "web");
  const manifestPath = path.join(root, "runtime_manifest.json");
  fs.mkdirSync(packageWorkDir, { recursive: true });
  fs.writeFileSync(manifestPath, JSON.stringify({
    platforms: {
      "mac15-arm64": {
        chromium: { directory: "chromium-1200", path: "runtime_trees/mac15-arm64/chromium-1200" },
      },
    },
  }));

  const cwd = process.cwd();
  process.chdir(packageWorkDir);
  try {
    assert.throws(() => materializePlaywrightBrowsers({
      browsers: ["firefox"],
      outDir: "playwright-browsers",
      runtimeManifest: manifestPath,
    }), /missing Playwright runtime entry for firefox on mac15-arm64/u);
  } finally {
    process.chdir(cwd);
  }
});

test("materialize-playwright-browsers passes extracted runtime tree paths into the copy helper", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-materialize-playwright-copy-helper-"));
  const packageWorkDir = path.join(root, "core", "apps", "web");
  const runtimeRoot = path.join(root, "playwright-browser-runtime");
  const manifestPath = path.join(runtimeRoot, "runtime_manifest.json");
  fs.mkdirSync(packageWorkDir, { recursive: true });
  writeFile(runtimeRoot, "runtime_trees/mac15-arm64/chromium-1200/chrome", "", 0o755);
  fs.writeFileSync(manifestPath, JSON.stringify({
    platforms: {
      "mac15-arm64": {
        chromium: { directory: "chromium-1200", path: "runtime_trees/mac15-arm64/chromium-1200" },
      },
    },
  }));

  const copyRuntimeTreeImpl = createSpy((sourceDir, targetDir) => {
    fs.mkdirSync(targetDir, { recursive: true });
    fs.writeFileSync(path.join(targetDir, "chrome"), "");
  });
  const cwd = process.cwd();
  process.chdir(packageWorkDir);
  try {
    materializePlaywrightBrowsers({
      browsers: ["chromium"],
      outDir: "playwright-browsers",
      runtimeManifest: manifestPath,
    }, { copyRuntimeTreeImpl });
  } finally {
    process.chdir(cwd);
  }

  assert.deepEqual(copyRuntimeTreeImpl.calls, [[
    path.join(runtimeRoot, "runtime_trees", "mac15-arm64", "chromium-1200"),
    path.join(packageWorkDir, "playwright-browsers", "mac15-arm64", "chromium-1200"),
  ]]);
});
