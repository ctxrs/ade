#!/usr/bin/env node

const CHANNEL_VALUES = new Set(["prod", "staging", "dev"]);
const PROFILE_VALUES = new Set(["parity", "override", "source-all"]);
const SURFACE_VALUES = new Set(["desktop", "daemon-web"]);

const resolveLaunchMode = ({
  channel = process.env.CTX_DESKTOP_CHANNEL || "dev",
  profile = process.env.CTX_RUNTIME_PROFILE || "parity",
  surface = process.env.CTX_LAUNCH_SURFACE || "desktop",
} = {}) => {
  const normalizedChannel = String(channel).trim() || "dev";
  const normalizedProfile = String(profile).trim() || "parity";
  const normalizedSurface = String(surface).trim() || "desktop";

  if (!CHANNEL_VALUES.has(normalizedChannel)) {
    throw new Error(
      `invalid CTX_DESKTOP_CHANNEL='${normalizedChannel}' (expected prod|staging|dev)`,
    );
  }
  if (!PROFILE_VALUES.has(normalizedProfile)) {
    throw new Error(
      `invalid CTX_RUNTIME_PROFILE='${normalizedProfile}' (expected parity|override|source-all)`,
    );
  }
  if (!SURFACE_VALUES.has(normalizedSurface)) {
    throw new Error(
      `invalid CTX_LAUNCH_SURFACE='${normalizedSurface}' (expected desktop|daemon-web)`,
    );
  }

  return {
    channel: normalizedChannel,
    profile: normalizedProfile,
    surface: normalizedSurface,
  };
};

module.exports = {
  resolveLaunchMode,
  CHANNEL_VALUES,
  PROFILE_VALUES,
  SURFACE_VALUES,
};

