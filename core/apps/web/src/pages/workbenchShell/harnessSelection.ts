import type { ProviderOptions, ProviderStatus } from "../../api/client";
import { hasConfiguredHarnessAuth } from "../../utils/providerAuthStatus";

export function getHarnessMruStorageKey(workspaceId: string): string {
  return `wb.harnessMru.${workspaceId}`;
}

export function collectSelectableHarnessProviderIds(
  providersById: Record<string, ProviderStatus>,
): string[] {
  return Object.values(providersById)
    .filter(
      (provider) =>
        provider.installed === true
        && provider.health === "ok"
        && provider.details?.ui_hidden !== "true",
    )
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
