const { resolveBoolishFlag } = require("./lib/boolish.cjs");

const shouldBundleRemoteDaemons = (env = process.env) => {
  return resolveBoolishFlag(env.CTX_BUNDLE_REMOTE_DAEMONS, true, "CTX_BUNDLE_REMOTE_DAEMONS");
};

module.exports = {
  shouldBundleRemoteDaemons,
};
