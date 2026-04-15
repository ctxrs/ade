const childProcess = require("node:child_process");
const path = require("node:path");

const DEFAULT_INFISICAL_ENV = "prod";
const CORE_ROOT = path.resolve(__dirname, "..", "..");

function readEnvOrInfisical(name, { env = process.env, infisicalEnv = DEFAULT_INFISICAL_ENV } = {}) {
  const direct = String(env[name] || "").trim();
  if (direct) {
    return direct;
  }
  const result = childProcess.spawnSync(
    "infisical",
    ["secrets", "get", name, "--plain", "--env", infisicalEnv],
    {
      cwd: CORE_ROOT,
      encoding: "utf8",
      env,
    },
  );
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    const stderr = String(result.stderr || "").trim();
    throw new Error(stderr || `failed to resolve ${name} from Infisical`);
  }
  return String(result.stdout || "").trim();
}

function resolveBuildBuddyApiKey({ env = process.env, infisicalEnv = DEFAULT_INFISICAL_ENV } = {}) {
  return (
    String(env.BUILD_BUDDY_API_KEY || env.BUILDBUDDY_API_KEY || "").trim()
    || readEnvOrInfisical("BUILD_BUDDY_API_KEY", { env, infisicalEnv })
  );
}

module.exports = {
  DEFAULT_INFISICAL_ENV,
  readEnvOrInfisical,
  resolveBuildBuddyApiKey,
};
