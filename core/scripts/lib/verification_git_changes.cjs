const childProcess = require("node:child_process");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..", "..");
const repoRoot = path.resolve(coreRoot, "..");
const DEFAULT_BASE_REF = "origin/main";

function normalizeRepoRelativePath(value) {
  return String(value || "")
    .trim()
    .replace(/\\/gu, "/")
    .replace(/^\.\//u, "")
    .replace(/^\/+/u, "");
}

function uniqueNormalizedPaths(entries) {
  return [...new Set(entries.map(normalizeRepoRelativePath).filter(Boolean))];
}

function formatGitError(args, error) {
  const stderr = String(error?.stderr || "").trim();
  if (stderr) {
    return `git ${args.join(" ")} failed: ${stderr}`;
  }
  return `git ${args.join(" ")} failed: ${String(error?.message || error)}`;
}

function execGitLines(args, { cwd = repoRoot } = {}) {
  try {
    const output = childProcess.execFileSync("git", args, {
      cwd,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
    return uniqueNormalizedPaths(output.split(/\r?\n/u));
  } catch (error) {
    throw new Error(formatGitError(args, error));
  }
}

function execGitValue(args, { cwd = repoRoot } = {}) {
  try {
    return childProcess.execFileSync("git", args, {
      cwd,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    }).trim();
  } catch (error) {
    throw new Error(formatGitError(args, error));
  }
}

function resolveDirtyFiles({ cwd = repoRoot } = {}) {
  return execGitLines(["diff", "--name-only"], { cwd });
}

function resolveStagedFiles({ cwd = repoRoot } = {}) {
  return execGitLines(["diff", "--name-only", "--cached"], { cwd });
}

function resolveUntrackedFiles({ cwd = repoRoot } = {}) {
  return execGitLines(["ls-files", "--others", "--exclude-standard"], { cwd });
}

function resolveWorkingTreeFiles({ cwd = repoRoot } = {}) {
  return uniqueNormalizedPaths([
    ...resolveDirtyFiles({ cwd }),
    ...resolveStagedFiles({ cwd }),
    ...resolveUntrackedFiles({ cwd }),
  ]);
}

function resolveMergeBase(baseRef = DEFAULT_BASE_REF, { cwd = repoRoot } = {}) {
  const normalizedBaseRef = String(baseRef || "").trim();
  if (!normalizedBaseRef) {
    throw new Error("a base ref is required to resolve merge-base changes");
  }
  return execGitValue(["merge-base", "HEAD", normalizedBaseRef], { cwd });
}

function resolveMergeBaseFiles(baseRef = DEFAULT_BASE_REF, { cwd = repoRoot } = {}) {
  const mergeBase = resolveMergeBase(baseRef, { cwd });
  return {
    baseRef: String(baseRef || "").trim(),
    mergeBase,
    changedFiles: execGitLines(["diff", "--name-only", `${mergeBase}...HEAD`], { cwd }),
  };
}

function enforceCleanWorkingTree({ cwd = repoRoot } = {}) {
  const changedFiles = resolveWorkingTreeFiles({ cwd });
  if (changedFiles.length > 0) {
    throw new Error(
      [
        "verify:merge-ready requires a clean worktree.",
        "Commit or stash these paths first:",
        ...changedFiles.map((entry) => `- ${entry}`),
      ].join("\n"),
    );
  }
  return changedFiles;
}

function resolveIntentChangeSet({
  cwd = repoRoot,
  intent,
  baseRef = DEFAULT_BASE_REF,
  changedFiles = [],
} = {}) {
  if (intent === "merge-ready") {
    enforceCleanWorkingTree({ cwd });
  }

  const explicitChangedFiles = uniqueNormalizedPaths(changedFiles);
  if (explicitChangedFiles.length > 0) {
    return {
      baseRef: String(baseRef || "").trim() || DEFAULT_BASE_REF,
      changedFiles: explicitChangedFiles,
      mergeBase: "",
      sources: {
        explicitChangedFiles,
      },
    };
  }

  if (intent === "merge-ready") {
    const { changedFiles: branchFiles, mergeBase } = resolveMergeBaseFiles(baseRef, { cwd });
    return {
      baseRef: String(baseRef || "").trim() || DEFAULT_BASE_REF,
      changedFiles: branchFiles,
      mergeBase,
      sources: {
        branchFiles,
        dirtyFiles: [],
        stagedFiles: [],
        untrackedFiles: [],
      },
    };
  }

  const dirtyFiles = resolveDirtyFiles({ cwd });
  const stagedFiles = resolveStagedFiles({ cwd });
  const untrackedFiles = resolveUntrackedFiles({ cwd });

  if (intent === "touched") {
    return {
      baseRef: String(baseRef || "").trim() || DEFAULT_BASE_REF,
      changedFiles: uniqueNormalizedPaths([...dirtyFiles, ...stagedFiles, ...untrackedFiles]),
      mergeBase: "",
      sources: {
        dirtyFiles,
        stagedFiles,
        untrackedFiles,
      },
    };
  }

  if (intent === "affected" || intent === "broader") {
    const { changedFiles: branchFiles, mergeBase } = resolveMergeBaseFiles(baseRef, { cwd });
    return {
      baseRef: String(baseRef || "").trim() || DEFAULT_BASE_REF,
      changedFiles: uniqueNormalizedPaths([
        ...branchFiles,
        ...dirtyFiles,
        ...stagedFiles,
        ...untrackedFiles,
      ]),
      mergeBase,
      sources: {
        branchFiles,
        dirtyFiles,
        stagedFiles,
        untrackedFiles,
      },
    };
  }

  throw new Error(`unsupported verification intent: ${intent}`);
}

module.exports = {
  DEFAULT_BASE_REF,
  enforceCleanWorkingTree,
  execGitLines,
  execGitValue,
  resolveDirtyFiles,
  resolveIntentChangeSet,
  resolveMergeBase,
  resolveMergeBaseFiles,
  resolveStagedFiles,
  resolveUntrackedFiles,
  resolveWorkingTreeFiles,
  uniqueNormalizedPaths,
};
