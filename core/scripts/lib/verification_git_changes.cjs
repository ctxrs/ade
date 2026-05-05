const childProcess = require("node:child_process");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..", "..");
const repoRoot = path.resolve(coreRoot, "..");
const DEFAULT_AGENT_BASE_REF = "origin/dev";
const DEFAULT_PROMOTION_BASE_REF = "origin/main";
const DEFAULT_BASE_REF = DEFAULT_PROMOTION_BASE_REF;

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
  if (args[0] === "merge-base" && args.length >= 3) {
    const baseRef = args[2];
    const details = stderr || String(error?.message || error);
    return [
      `git ${args.join(" ")} failed: ${details}`,
      `Unable to resolve verification base ref '${baseRef}'. Fetch that ref or pass --base explicitly.`,
    ].join("\n");
  }
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

function defaultBaseRefForIntent(intent, env = process.env) {
  const agentBaseRef = String(env.CTX_VERIFY_AGENT_BASE_REF || "").trim();
  const promotionBaseRef = String(env.CTX_VERIFY_PROMOTION_BASE_REF || "").trim();
  switch (intent) {
    case "affected":
    case "broader":
      return agentBaseRef || DEFAULT_AGENT_BASE_REF;
    case "merge-ready":
      return promotionBaseRef || DEFAULT_PROMOTION_BASE_REF;
    case "touched":
      return "";
    default:
      throw new Error(`unsupported verification intent: ${intent}`);
  }
}

function resolveBaseRefForIntent(intent, baseRef, env = process.env) {
  return String(baseRef || "").trim() || defaultBaseRefForIntent(intent, env);
}

function resolveIntentChangeSet({
  cwd = repoRoot,
  intent,
  baseRef = "",
  changedFiles = [],
  env = process.env,
} = {}) {
  const resolvedBaseRef = resolveBaseRefForIntent(intent, baseRef, env);

  if (intent === "merge-ready") {
    enforceCleanWorkingTree({ cwd });
  }

  const explicitChangedFiles = uniqueNormalizedPaths(changedFiles);
  if (explicitChangedFiles.length > 0) {
    return {
      baseRef: resolvedBaseRef,
      changedFiles: explicitChangedFiles,
      mergeBase: "",
      sources: {
        explicitChangedFiles,
      },
    };
  }

  if (intent === "merge-ready") {
    const { changedFiles: branchFiles, mergeBase } = resolveMergeBaseFiles(resolvedBaseRef, { cwd });
    return {
      baseRef: resolvedBaseRef,
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
      baseRef: resolvedBaseRef,
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
    const { changedFiles: branchFiles, mergeBase } = resolveMergeBaseFiles(resolvedBaseRef, { cwd });
    return {
      baseRef: resolvedBaseRef,
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
  DEFAULT_AGENT_BASE_REF,
  DEFAULT_BASE_REF,
  DEFAULT_PROMOTION_BASE_REF,
  defaultBaseRefForIntent,
  enforceCleanWorkingTree,
  execGitLines,
  execGitValue,
  resolveBaseRefForIntent,
  resolveDirtyFiles,
  resolveIntentChangeSet,
  resolveMergeBase,
  resolveMergeBaseFiles,
  resolveStagedFiles,
  resolveUntrackedFiles,
  resolveWorkingTreeFiles,
  uniqueNormalizedPaths,
};
