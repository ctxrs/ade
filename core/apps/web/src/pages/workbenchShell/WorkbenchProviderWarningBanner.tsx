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
    title: `${flagged.length} provider runtime${flagged.length === 1 ? "" : "s"} need${flagged.length === 1 ? "s" : ""} an update.`,
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
  const [updateAllSuppressed, setUpdateAllSuppressed] = useState(false);

  useEffect(() => {
    setDismissedSignature(readDismissedWarningSignature(workspaceId));
  }, [workspaceId, warning?.signature]);

  if (!warning || dismissedSignature === warning.signature || updateAllSuppressed) return null;

  const dismiss = () => {
    persistDismissedWarningSignature(workspaceId, warning.signature);
    setDismissedSignature(warning.signature);
  };

  const handleOpenSettings = () => {
    dismiss();
    onOpenSettings();
  };

  const handleUpdateAll = async () => {
    setUpdateAllSuppressed(true);
    try {
      await onUpdateProviders(warning.installableProviderIds);
    } catch {
      // The workbench already surfaces install errors independently.
    } finally {
      setUpdateAllSuppressed(false);
    }
  };

  return (
    <div
      className="wb-snackbar wb-provider-warning-snackbar"
      role="status"
      aria-live="polite"
      aria-labelledby="wb-provider-warning-title"
      data-testid="workbench-provider-warning"
    >
      <div className="wb-provider-warning-header">
        <div className="wb-snackbar-title" id="wb-provider-warning-title">{warning.title}</div>
        <button
          type="button"
          className="wb-snackbar-close"
          onClick={dismiss}
          aria-label="Dismiss"
        >
          <X size={16} aria-hidden="true" />
        </button>
      </div>
      <div className="wb-snackbar-actions wb-provider-warning-actions">
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
  );
}
