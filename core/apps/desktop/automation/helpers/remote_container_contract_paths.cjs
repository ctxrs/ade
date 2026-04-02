const trimTrailingSlashes = (value) => String(value || "").replace(/\/+$/, "");

const expectedRemoteRootPrefix = ({
  remoteBase,
  remoteDataDir,
  container,
  sourceKind,
}) => {
  if (container === "sandbox" && (sourceKind === "clone" || sourceKind === "new")) {
    return `${trimTrailingSlashes(remoteDataDir)}/workspaces/staging/`;
  }
  return `${trimTrailingSlashes(remoteBase)}/`;
};

module.exports = {
  expectedRemoteRootPrefix,
};
