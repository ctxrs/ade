#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");
const childProcess = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const scriptPath = path.join(repoRoot, "tools", "bazel", "run_workspace_task.sh");
const scriptText = fs.readFileSync(scriptPath, "utf8");

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

test("run_workspace_task copies the workspace without requiring rsync", () => {
  assert.doesNotMatch(scriptText, /\brsync\b/, "workspace task wrapper should not depend on rsync");

  const tempRoot = makeVolatileTempDir("ctx-run-workspace-task-");
  const runfilesDir = path.join(tempRoot, "runfiles");
  const runfilesRepo = path.join(runfilesDir, "_main");
  const realWorkspace = path.join(tempRoot, "real-workspace");

  fs.mkdirSync(path.join(runfilesRepo, "core", "apps", "web"), { recursive: true });
  fs.mkdirSync(path.join(runfilesRepo, ".git"), { recursive: true });
  fs.mkdirSync(path.join(runfilesRepo, "node_modules"), { recursive: true });
  fs.mkdirSync(path.join(runfilesRepo, "core", "target"), { recursive: true });
  fs.mkdirSync(path.join(runfilesRepo, "core", "apps", "web", "dist"), { recursive: true });

  fs.writeFileSync(path.join(runfilesRepo, "core", "package.json"), "{}\n");
  fs.writeFileSync(path.join(runfilesRepo, "keep.txt"), "repo-root\n");
  fs.writeFileSync(path.join(runfilesRepo, ".git", "HEAD"), "ref: refs/heads/main\n");
  fs.writeFileSync(path.join(runfilesRepo, "node_modules", "excluded.txt"), "exclude me\n");
  fs.writeFileSync(path.join(runfilesRepo, "core", "target", "excluded.txt"), "exclude me\n");
  fs.writeFileSync(path.join(runfilesRepo, "core", "apps", "web", "web_keep.txt"), "web\n");
  fs.writeFileSync(path.join(runfilesRepo, "core", "apps", "web", "dist", "excluded.txt"), "exclude me\n");

  fs.mkdirSync(path.join(realWorkspace, "core", "node_modules"), { recursive: true });
  fs.mkdirSync(path.join(realWorkspace, "core", "apps", "web", "node_modules"), { recursive: true });
  fs.writeFileSync(path.join(realWorkspace, "core", "package.json"), "{}\n");
  fs.writeFileSync(path.join(realWorkspace, "core", "node_modules", "marker.txt"), "root-node-modules\n");
  fs.writeFileSync(
    path.join(realWorkspace, "core", "apps", "web", "node_modules", "marker.txt"),
    "web-node-modules\n",
  );

  const binDir = path.join(tempRoot, "bin");
  fs.mkdirSync(binDir, { recursive: true });
  for (const commandName of ["bash", "cat", "dirname", "ln", "mkdir", "mktemp", "rm", "tar", "which"]) {
    symlinkBinary(binDir, commandName);
  }

  const checkScript = [
    "test -f ../../../keep.txt",
    "test -f ./web_keep.txt",
    "test ! -e ../../../.git",
    "test ! -e ../../../node_modules",
    "test ! -e ../../target",
    "test ! -e ./dist",
    "test -L ../../node_modules",
    "test -L ./node_modules",
    'test "$(cat ../../node_modules/marker.txt)" = "root-node-modules"',
    'test "$(cat ./node_modules/marker.txt)" = "web-node-modules"',
  ].join(" && ");

  childProcess.execFileSync("bash", [scriptPath, "core/apps/web", "bash", "-lc", checkScript], {
    cwd: repoRoot,
    env: {
      ...process.env,
      BUILD_WORKSPACE_DIRECTORY: realWorkspace,
      RUNFILES_DIR: runfilesDir,
      PATH: binDir,
      TMPDIR: tempRoot,
    },
    stdio: "pipe",
  });
});
