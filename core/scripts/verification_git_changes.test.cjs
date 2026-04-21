const assert = require("node:assert/strict");
const childProcess = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  enforceCleanWorkingTree,
  resolveMergeBaseFiles,
  resolveIntentChangeSet,
} = require("./lib/verification_git_changes.cjs");

function runGit(cwd, args) {
  childProcess.execFileSync("git", args, {
    cwd,
    stdio: ["ignore", "pipe", "pipe"],
  });
}

function writeFile(root, relativePath, contents) {
  const absolutePath = path.join(root, relativePath);
  fs.mkdirSync(path.dirname(absolutePath), { recursive: true });
  fs.writeFileSync(absolutePath, contents, "utf8");
}

function createTempRepo() {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), "verification-git-changes-"));
  runGit(repoRoot, ["init"]);
  runGit(repoRoot, ["config", "user.email", "ctx@example.com"]);
  runGit(repoRoot, ["config", "user.name", "ctx"]);
  writeFile(repoRoot, "README.md", "initial\n");
  runGit(repoRoot, ["add", "README.md"]);
  runGit(repoRoot, ["commit", "-m", "initial"]);
  runGit(repoRoot, ["branch", "-M", "main"]);
  runGit(repoRoot, ["checkout", "-b", "feature"]);
  return repoRoot;
}

test("touched and affected change sets keep their diff semantics separate", () => {
  const repoRoot = createTempRepo();

  writeFile(repoRoot, "committed.txt", "committed\n");
  runGit(repoRoot, ["add", "committed.txt"]);
  runGit(repoRoot, ["commit", "-m", "feature commit"]);

  writeFile(repoRoot, "dirty.txt", "dirty\n");
  writeFile(repoRoot, "staged.txt", "staged\n");
  runGit(repoRoot, ["add", "staged.txt"]);
  writeFile(repoRoot, "untracked.txt", "untracked\n");

  const touched = resolveIntentChangeSet({
    cwd: repoRoot,
    intent: "touched",
    baseRef: "main",
  });
  const affected = resolveIntentChangeSet({
    cwd: repoRoot,
    intent: "affected",
    baseRef: "main",
  });

  assert.deepEqual([...touched.changedFiles].sort(), [
    "dirty.txt",
    "staged.txt",
    "untracked.txt",
  ]);
  assert.equal(touched.mergeBase, "");

  assert.deepEqual([...affected.changedFiles].sort(), [
    "committed.txt",
    "dirty.txt",
    "staged.txt",
    "untracked.txt",
  ]);
  assert.ok(affected.mergeBase.length > 0);
});

test("merge-ready enforces a clean worktree and then resolves committed branch changes", () => {
  const repoRoot = createTempRepo();
  writeFile(repoRoot, "committed.txt", "committed\n");
  runGit(repoRoot, ["add", "committed.txt"]);
  runGit(repoRoot, ["commit", "-m", "feature commit"]);
  writeFile(repoRoot, "dirty.txt", "dirty\n");

  assert.throws(() => enforceCleanWorkingTree({ cwd: repoRoot }), /verify:merge-ready requires a clean worktree/u);

  fs.rmSync(path.join(repoRoot, "dirty.txt"));

  const mergeReady = resolveIntentChangeSet({
    cwd: repoRoot,
    intent: "merge-ready",
    baseRef: "main",
  });

  assert.deepEqual(mergeReady.changedFiles, ["committed.txt"]);
  assert.ok(mergeReady.mergeBase.length > 0);
});

test("merge-ready still enforces a clean worktree when explicit changed files are supplied", () => {
  const repoRoot = createTempRepo();
  writeFile(repoRoot, "dirty.txt", "dirty\n");

  assert.throws(() => resolveIntentChangeSet({
    cwd: repoRoot,
    intent: "merge-ready",
    baseRef: "main",
    changedFiles: ["README.md"],
  }), /verify:merge-ready requires a clean worktree/u);
});

test("merge-base file resolution excludes unrelated dirty worktree changes", () => {
  const repoRoot = createTempRepo();

  writeFile(repoRoot, "committed.txt", "committed\n");
  runGit(repoRoot, ["add", "committed.txt"]);
  runGit(repoRoot, ["commit", "-m", "feature commit"]);
  writeFile(repoRoot, "dirty.txt", "dirty\n");

  const mergeBaseFiles = resolveMergeBaseFiles("main", { cwd: repoRoot });

  assert.deepEqual(mergeBaseFiles.changedFiles, ["committed.txt"]);
  assert.ok(mergeBaseFiles.mergeBase.length > 0);
});
