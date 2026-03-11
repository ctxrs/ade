import type { ProviderOptions, ProviderStatus } from "../../api/client";
import { isInstalledVisibleHarnessProviderStatus } from "../../utils/providerInventory";
import { hasConfiguredHarnessAuth } from "../../utils/providerAuthStatus";

export function getHarnessMruStorageKey(workspaceId: string): string {
  return `wb.harnessMru.${workspaceId}`;
}

export function collectSelectableHarnessProviderIds(
  providersById: Record<string, ProviderStatus>,
): string[] {
  return Object.values(providersById)
    .filter((provider) => isInstalledVisibleHarnessProviderStatus(provider))
    .map((provider) => provider.provider_id);
}

type ResolveInitialHarnessSelectionArgs = {
  providerIds: string[];
  providerOptions: Record<string, ProviderOptions | undefined>;
  mruProviderId?: string | null;
};

export function resolveInitialHarnessSelection({
  providerIds,
  providerOptions,
  mruProviderId,
}: ResolveInitialHarnessSelectionArgs): string | null {
  const authedProviderIds = providerIds.filter((providerId) =>
    hasConfiguredHarnessAuth(providerId, providerOptions[providerId]),
  );

  const mru = (mruProviderId ?? "").trim();
  if (mru && authedProviderIds.includes(mru)) {
    return mru;
  }

  if (authedProviderIds.length === 1) {
    return authedProviderIds[0];
  }

  return null;
}

export function shouldFinalizeInitialHarnessSelection(
  selectedProviderId: string | null,
): selectedProviderId is string {
  return selectedProviderId !== null;
}
