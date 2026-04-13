import { useEffect, useMemo, useState } from "react";
import { X } from "lucide-react";
import type { ProviderStatus } from "../../api/client";
import { providerDetailFlag } from "../../utils/boolish";
import { HARNESS_CATALOG } from "../../utils/harnessCatalog";
import { isVisibleHarnessProviderStatus, providerUsabilityReason } from "../../utils/providerInventory";
import { formatProviderVersionDisplay, getMatrixVersionDisplay } from "../../utils/providerVersionLabel";

type WorkbenchProviderWarningBannerProps = {
  workspaceId: string;
  providersById: Record<string, ProviderStatus>;
  updateAllBusy?: boolean;
  onUpdateProviders: (providerIds: string[]) => Promise<void> | void;
  onOpenSettings: () => void;
};

type WarningProvider = {
  providerId: string;
  label: string;
  installSupported: boolean;
  installedVersion: string | null;
  recommendedVersion: string | null;
  reason: string | null;
};

export type WorkbenchProviderWarning = {
  signature: string;
  title: string;
  subtitle: string;
  providers: WarningProvider[];
  installableProviderIds: string[];
};

const HARNESS_LABELS = new Map(HARNESS_CATALOG.map((entry) => [entry.id, entry.label]));
const DISMISSED_WARNING_SIGNATURE_STORAGE_KEY_PREFIX = "wb.provider_runtime_warning.dismissed";

const labelForProvider = (providerId: string): string =>
  HARNESS_LABELS.get(providerId) ?? providerId;

const summarizeProvider = (
  provider: ProviderStatus,
): WarningProvider | null => {
  if (!isVisibleHarnessProviderStatus(provider)) return null;
  const requiresRuntimeUpdate =
    provider.health === "unsupported_version"
    || providerDetailFlag(provider.details, "matrix_update_available")
    || providerDetailFlag(provider.details, "managed_dependency_update_available")
    || providerDetailFlag(provider.details, "managed_fingerprint_mismatch");
  if (!requiresRuntimeUpdate) return null;
  return {
    providerId: provider.provider_id,
    label: labelForProvider(provider.provider_id),
    installSupported: providerDetailFlag(provider.details, "install_supported"),
    installedVersion: formatProviderVersionDisplay(provider),
    recommendedVersion: getMatrixVersionDisplay(provider.details, "recommended"),
    reason: providerUsabilityReason(provider),
  };
};

const warningDismissalStorageKey = (workspaceId: string): string =>
  `${DISMISSED_WARNING_SIGNATURE_STORAGE_KEY_PREFIX}.${workspaceId}`;

const readDismissedWarningSignature = (workspaceId: string): string | null => {
  try {
    const stored = window.sessionStorage.getItem(warningDismissalStorageKey(workspaceId));
    return stored ? stored.trim() || null : null;
  } catch {
    return null;
  }
};

const persistDismissedWarningSignature = (workspaceId: string, signature: string): void => {
  try {
    window.sessionStorage.setItem(warningDismissalStorageKey(workspaceId), signature);
  } catch {
    // Ignore storage failures and keep the dismissal in memory for the current render.
  }
};

export const buildWorkbenchProviderWarning = (
  providersById: Record<string, ProviderStatus>,
): WorkbenchProviderWarning | null => {
  const flagged = Object.values(providersById)
    .map(summarizeProvider)
    .filter((provider): provider is WarningProvider => provider !== null)
    .sort((lhs, rhs) => lhs.label.localeCompare(rhs.label));

  if (flagged.length === 0) return null;

  const requiresAppUpdate = flagged.some((provider) => provider.reason?.includes("newer ctx build"));
  const installableProviderIds = flagged
    .filter((provider) => provider.installSupported)
    .map((provider) => provider.providerId);
  const signature = JSON.stringify(
    flagged.map((provider) => ({
      providerId: provider.providerId,
      installedVersion: provider.installedVersion,
      recommendedVersion: provider.recommendedVersion,
      installSupported: provider.installSupported,
      reason: provider.reason,
    })),
  );

  return {
    signature,
    title: requiresAppUpdate
      ? "Provider runtimes are not supported by this ctx build."
      : "Provider runtimes need an update.",
    subtitle: requiresAppUpdate
      ? "At least one installed runtime is pinned to a newer ctx build. Review the affected runtimes below."
      : "Update the pinned runtimes below or open settings for more detail.",
    providers: flagged,
    installableProviderIds,
  };
};

export function WorkbenchProviderWarningBanner({
  workspaceId,
  providersById,
  updateAllBusy = false,
  onUpdateProviders,
  onOpenSettings,
}: WorkbenchProviderWarningBannerProps) {
  const warning = useMemo(
    () => buildWorkbenchProviderWarning(providersById),
    [providersById],
  );
  const [dismissedSignature, setDismissedSignature] = useState<string | null>(() =>
    readDismissedWarningSignature(workspaceId));

  useEffect(() => {
    setDismissedSignature(readDismissedWarningSignature(workspaceId));
  }, [workspaceId, warning?.signature]);

  if (!warning || dismissedSignature === warning.signature) return null;

  const dismiss = () => {
    persistDismissedWarningSignature(workspaceId, warning.signature);
    setDismissedSignature(warning.signature);
  };

  const handleOpenSettings = () => {
    dismiss();
    onOpenSettings();
  };

  const handleUpdateAll = async () => {
    try {
      await onUpdateProviders(warning.installableProviderIds);
      dismiss();
    } catch {
      // The workbench already surfaces install errors. Keep the modal open on failure.
    }
  };

  return (
    <div className="modal-overlay" role="dialog" aria-modal="true" aria-labelledby="wb-provider-warning-title">
      <div className="modal wb-provider-warning-modal" onClick={(event) => event.stopPropagation()} data-testid="workbench-provider-warning">
        <div className="wb-provider-warning-header">
          <div className="wb-provider-warning-heading">
            <div className="wb-provider-warning-title" id="wb-provider-warning-title">{warning.title}</div>
            <div className="wb-provider-warning-copy">{warning.subtitle}</div>
          </div>
          <button
            type="button"
            className="settings-harness-modal-close"
            onClick={dismiss}
            aria-label="Dismiss"
          >
            <X size={16} aria-hidden="true" />
          </button>
        </div>
        <div className="wb-provider-warning-list">
          {warning.providers.map((provider) => {
            const installed = provider.installedVersion ?? "unknown";
            const recommended = provider.recommendedVersion ?? "pinned runtime";
            return (
              <div key={provider.providerId} className="wb-provider-warning-item">
                <div className="wb-provider-warning-item-row">
                  <div className="wb-provider-warning-item-title">{provider.label}</div>
                  {provider.installSupported ? (
                    <div className="wb-provider-warning-item-pill">Managed update</div>
                  ) : null}
                </div>
                <div className="wb-provider-warning-item-meta">
                  Installed {installed} · Expected {recommended}
                </div>
                {provider.reason ? (
                  <div className="wb-provider-warning-item-reason">{provider.reason}</div>
                ) : null}
              </div>
            );
          })}
        </div>
        <div className="modal-actions wb-provider-warning-actions">
          {warning.installableProviderIds.length > 0 ? (
            <button
              type="button"
              className="wb-snackbar-btn"
              onClick={() => {
                void handleUpdateAll();
              }}
              disabled={updateAllBusy}
            >
              {updateAllBusy ? "Updating…" : "Update All"}
            </button>
          ) : null}
          <button
            type="button"
            className="wb-snackbar-btn wb-snackbar-btn-secondary"
            onClick={handleOpenSettings}
          >
            Open Settings
          </button>
        </div>
      </div>
    </div>
  );
}
