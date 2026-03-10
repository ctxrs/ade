const TRUTHY_ENV_VALUES = new Set(["1", "true", "yes", "on"]);

export const parseCsv = (value: string | undefined): string[] =>
  String(value ?? "")
    .split(",")
    .map((entry) => entry.trim())
    .filter(Boolean);

export const envTruthy = (value: string | undefined): boolean =>
  TRUTHY_ENV_VALUES.has(String(value ?? "").trim().toLowerCase());

export const bundledOnlyModeAppliesToProvider = (
  providerId: string,
  env: NodeJS.ProcessEnv = process.env,
): boolean => {
  if (!envTruthy(env.CTX_E2E_BUNDLED_ONLY)) {
    return false;
  }

  const providers = parseCsv(env.CTX_E2E_BUNDLED_ONLY_PROVIDERS);
  if (providers.length === 0) {
    return true;
  }

  return providers.includes(providerId.trim());
};

export const shouldSkipBundledOnlyInstall = (
  providerId: string,
  env: NodeJS.ProcessEnv = process.env,
): boolean => {
  if (!envTruthy(env.CTX_E2E_INSTALL_SMOKE_SKIP_BUNDLED_ONLY_INSTALLS)) {
    return false;
  }

  return bundledOnlyModeAppliesToProvider(providerId, env);
};
