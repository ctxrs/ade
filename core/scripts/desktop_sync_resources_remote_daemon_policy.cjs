const normalizeToggle = (value) => String(value ?? "").trim().toLowerCase();

const shouldBundleRemoteDaemons = (env = process.env) => {
  const raw = normalizeToggle(env.CTX_BUNDLE_REMOTE_DAEMONS || "1");
  if (raw === "0" || raw === "false" || raw === "no" || raw === "off") {
    return false;
  }
  return true;
};

module.exports = {
  shouldBundleRemoteDaemons,
};
