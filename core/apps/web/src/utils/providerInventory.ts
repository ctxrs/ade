import type { ProviderStatus } from "../api/client";
import { providerDetailFlag } from "./boolish";

export function isVisibleHarnessProviderStatus(
  provider: ProviderStatus | null | undefined,
): provider is ProviderStatus {
  if (!provider) return false;
  return !providerDetailFlag(provider.details, "ui_hidden")
    && provider.details?.provider_kind !== "dependency";
}

export function isInstalledVisibleHarnessProviderStatus(
  provider: ProviderStatus | null | undefined,
): provider is ProviderStatus {
  return isVisibleHarnessProviderStatus(provider)
    && provider.installed === true
    && provider.health === "ok";
}
