#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");
const childProcess = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const scriptPath = path.join(repoRoot, "tools", "bazel", "run_node_task.sh");
const scriptText = fs.readFileSync(scriptPath, "utf8");
const bashPath = childProcess.execFileSync("which", ["bash"], {
  encoding: "utf8",
}).trim();

function makeVolatileTempDir(prefix) {
  const candidates = [
    process.env.CTX_VOLATILE_TMPDIR,
    path.join(os.homedir(), ".ctx", "volatile", "tmp"),
  ].filter(Boolean);
  for (const candidate of candidates) {
    try {
      fs.mkdirSync(candidate, { recursive: true });
      fs.accessSync(candidate, fs.constants.W_OK);
      return fs.mkdtempSync(path.join(candidate, prefix));
    } catch {}
  }
  throw new Error(`unable to allocate writable volatile temp dir from: ${candidates.join(", ")}`);
}

function symlinkBinary(binDir, commandName) {
  const resolved = childProcess.execFileSync("which", [commandName], {
    encoding: "utf8",
  }).trim();
  fs.symlinkSync(resolved, path.join(binDir, commandName));
}

test("run_node_task executes within the Bazel runfiles repo without mirroring the workspace", () => {
  assert.doesNotMatch(scriptText, /\brsync\b/);
  assert.doesNotMatch(scriptText, /\btar\b/);

  const tempRoot = makeVolatileTempDir("ctx-run-node-task-");
  const runfilesDir = path.join(tempRoot, "runfiles");
  const runfilesRepo = path.join(runfilesDir, "_main");
  const binDir = path.join(tempRoot, "bin");

  fs.mkdirSync(path.join(runfilesRepo, "core"), { recursive: true });
  fs.writeFileSync(path.join(runfilesRepo, "core", "package.json"), "{}\n");
  fs.mkdirSync(binDir, { recursive: true });
  symlinkBinary(binDir, "dirname");

  childProcess.execFileSync(
    bashPath,
    [
      scriptPath,
      "core",
      "-e",
      "require('node:fs').writeFileSync('cwd.txt', process.cwd())",
    ],
    {
      cwd: repoRoot,
      env: {
        ...process.env,
        NODE: process.execPath,
        PATH: binDir,
        RUNFILES_DIR: runfilesDir,
        TMPDIR: tempRoot,
      },
      stdio: "pipe",
    },
  );

  assert.equal(
    fs.readFileSync(path.join(runfilesRepo, "core", "cwd.txt"), "utf8"),
    path.join(runfilesRepo, "core"),
  );
});

test("run_node_task resolves repo root from a manifest-only RUNFILES_MANIFEST_FILE", () => {
  const tempRoot = makeVolatileTempDir("ctx-run-node-task-manifest-");
  const manifestRepo = path.join(tempRoot, "workspace with spaces");
  const manifestCore = path.join(manifestRepo, "core");
  const manifestPath = path.join(tempRoot, "tool.runfiles_manifest");
  const binDir = path.join(tempRoot, "bin");

  assert.equal(fs.existsSync(path.join(tempRoot, "tool.runfiles")), false);
  fs.mkdirSync(manifestCore, { recursive: true });
  fs.writeFileSync(path.join(manifestCore, "package.json"), "{}\n");
  fs.writeFileSync(
    manifestPath,
    `_main/core/package.json ${path.join(manifestCore, "package.json")}\n`,
  );
  fs.mkdirSync(binDir, { recursive: true });
  symlinkBinary(binDir, "dirname");

  childProcess.execFileSync(
    bashPath,
    [
      scriptPath,
      "core",
      "-e",
      "require('node:fs').writeFileSync('cwd-manifest.txt', process.cwd())",
    ],
    {
      cwd: repoRoot,
      env: {
        ...process.env,
        NODE: process.execPath,
        PATH: binDir,
        RUNFILES_MANIFEST_FILE: manifestPath,
        TMPDIR: tempRoot,
      },
      stdio: "pipe",
    },
  );

  assert.equal(
    fs.readFileSync(path.join(manifestCore, "cwd-manifest.txt"), "utf8"),
    manifestCore,
  );
});

test("run_node_task resolves repo root from launcher-adjacent runfiles", () => {
  const tempRoot = makeVolatileTempDir("ctx-run-node-task-launcher-");
  const launcherPath = path.join(tempRoot, "desktop_launch_mode_contracts");
  const runfilesDir = `${launcherPath}.runfiles`;
  const runfilesRepo = path.join(runfilesDir, "_main");
  const binDir = path.join(tempRoot, "bin");

  fs.symlinkSync(scriptPath, launcherPath);
  fs.mkdirSync(path.join(runfilesRepo, "core"), { recursive: true });
  fs.writeFileSync(path.join(runfilesRepo, "core", "package.json"), "{}\n");
  fs.mkdirSync(binDir, { recursive: true });
  symlinkBinary(binDir, "dirname");

  childProcess.execFileSync(
    bashPath,
    [
      launcherPath,
      "core",
      "-e",
      "require('node:fs').writeFileSync('cwd-launcher.txt', process.cwd())",
    ],
    {
      cwd: repoRoot,
      env: {
        ...process.env,
        NODE: process.execPath,
        PATH: binDir,
        TMPDIR: tempRoot,
      },
      stdio: "pipe",
    },
  );

  assert.equal(
    fs.readFileSync(path.join(runfilesRepo, "core", "cwd-launcher.txt"), "utf8"),
    path.join(runfilesRepo, "core"),
  );
});

test("run_node_task fails closed when Bazel runfiles are unavailable", () => {
  const tempRoot = makeVolatileTempDir("ctx-run-node-task-workspace-");
  const binDir = path.join(tempRoot, "bin");

  fs.mkdirSync(binDir, { recursive: true });
  symlinkBinary(binDir, "dirname");

  const result = childProcess.spawnSync(
    bashPath,
    [scriptPath, "core", "-e", "process.stdout.write(process.cwd())"],
    {
      cwd: repoRoot,
      env: {
        ...process.env,
        BUILD_WORKSPACE_DIRECTORY: path.join(tempRoot, "workspace"),
        NODE: process.execPath,
        PATH: binDir,
        TMPDIR: tempRoot,
      },
      stdio: "pipe",
    },
  );

  assert.notEqual(result.status, 0);
  assert.match(result.stderr.toString(), /failed to locate Bazel repo root/);
});
