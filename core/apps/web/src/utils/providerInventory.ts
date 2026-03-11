import type { ProviderStatus } from "../api/client";

export function isVisibleHarnessProviderStatus(
  provider: ProviderStatus | null | undefined,
): provider is ProviderStatus {
  if (!provider) return false;
  return provider.details?.ui_hidden !== "true"
    && provider.details?.provider_kind !== "dependency";
}

export function isInstalledVisibleHarnessProviderStatus(
  provider: ProviderStatus | null | undefined,
): provider is ProviderStatus {
  return isVisibleHarnessProviderStatus(provider)
    && provider.installed === true
    && provider.health === "ok";
}
