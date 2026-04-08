const { resolveCtxCacheLayout, resolveRepoScopeKey } = require("./cache_roots.cjs");

function resolveCargoTargetDir({ cwd, env = process.env } = {}) {
  return resolveCtxCacheLayout({
    cwd: cwd || process.cwd(),
    env,
  }).workspaceCargoTargetDir;
}

module.exports = {
  resolveGitDirName: resolveRepoScopeKey,
  resolveCargoTargetDir,
};
