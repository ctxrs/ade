const childProcess = require("node:child_process");
const os = require("node:os");
const path = require("node:path");

function resolveGitDirName(cwd) {
  try {
    const raw = childProcess
      .execSync("git rev-parse --git-dir", {
        cwd,
        stdio: ["ignore", "pipe", "ignore"],
      })
      .toString()
      .trim();
    if (!raw) {
      return "default";
    }
    return path.basename(raw);
  } catch {
    return "default";
  }
}

function resolveCargoTargetDir({ cwd, env = process.env } = {}) {
  const raw = String(env.CARGO_TARGET_DIR || "").trim();
  if (raw) {
    return raw;
  }
  const effectiveCwd = cwd || process.cwd();
  return path.join(
    os.homedir(),
    ".cache",
    "cargo",
    "ctx-monorepo",
    resolveGitDirName(effectiveCwd),
  );
}

module.exports = {
  resolveGitDirName,
  resolveCargoTargetDir,
};
