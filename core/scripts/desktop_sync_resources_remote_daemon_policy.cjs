const { resolveBoolishFlag } = require("./lib/boolish.cjs");

const DEFAULT_REMOTE_DAEMON_BUNDLE_ARCHES = Object.freeze(["x86_64", "aarch64"]);

const shouldBundleRemoteDaemons = (env = process.env) => {
  return resolveBoolishFlag(env.CTX_BUNDLE_REMOTE_DAEMONS, true, "CTX_BUNDLE_REMOTE_DAEMONS");
};

const normalizeRemoteDaemonBundleArch = (raw) => {
  const value = String(raw || "").trim().toLowerCase();
  switch (value) {
    case "x64":
    case "amd64":
    case "x86_64":
    case "linux-x64":
    case "linux/amd64":
    case "linux/x86_64":
      return "x86_64";
    case "arm64":
    case "aarch64":
    case "linux-arm64":
    case "linux/arm64":
    case "linux/aarch64":
      return "aarch64";
    default:
      throw new Error(
        `Invalid CTX_BUNDLE_REMOTE_DAEMON_ARCHES entry "${raw}". Expected one of x86_64,aarch64,linux-x64,linux-arm64.`,
      );
  }
};

const resolveRemoteDaemonBundleTargetArches = (env = process.env) => {
  const raw = String(env.CTX_BUNDLE_REMOTE_DAEMON_ARCHES || "").trim();
  if (!raw) {
    return [...DEFAULT_REMOTE_DAEMON_BUNDLE_ARCHES];
  }

  const targets = [];
  for (const entry of raw.split(",")) {
    const arch = normalizeRemoteDaemonBundleArch(entry);
    if (!targets.includes(arch)) {
      targets.push(arch);
    }
  }
  if (targets.length === 0) {
    throw new Error("CTX_BUNDLE_REMOTE_DAEMON_ARCHES must include at least one target arch");
  }
  return targets;
};

module.exports = {
  DEFAULT_REMOTE_DAEMON_BUNDLE_ARCHES,
  resolveRemoteDaemonBundleTargetArches,
  shouldBundleRemoteDaemons,
};
