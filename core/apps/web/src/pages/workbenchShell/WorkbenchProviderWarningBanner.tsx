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
  title: string;
  providers: WarningProvider[];
  installableProviderIds: string[];
};

const HARNESS_LABELS = new Map(HARNESS_CATALOG.map((entry) => [entry.id, entry.label]));
const ACKNOWLEDGED_WARNING_PROVIDER_IDS_STORAGE_KEY_PREFIX = "wb.provider_runtime_warning.acknowledged_provider_ids";

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

const warningAcknowledgementStorageKey = (workspaceId: string): string =>
  `${ACKNOWLEDGED_WARNING_PROVIDER_IDS_STORAGE_KEY_PREFIX}.${workspaceId}`;

const normalizeProviderIds = (providerIds: string[]): string[] =>
  Array.from(new Set(providerIds.map((providerId) => providerId.trim()).filter(Boolean))).sort();

const readAcknowledgedWarningProviderIds = (workspaceId: string): string[] => {
  try {
    const stored = window.sessionStorage.getItem(warningAcknowledgementStorageKey(workspaceId));
    if (!stored) return [];
    const parsed = JSON.parse(stored);
    if (!Array.isArray(parsed)) return [];
    return normalizeProviderIds(parsed.filter((value): value is string => typeof value === "string"));
  } catch {
    return [];
  }
};

const persistAcknowledgedWarningProviderIds = (workspaceId: string, providerIds: string[]): void => {
  try {
    window.sessionStorage.setItem(
      warningAcknowledgementStorageKey(workspaceId),
      JSON.stringify(normalizeProviderIds(providerIds)),
    );
  } catch {
    // Ignore storage failures and keep the acknowledgement in memory for the current render.
  }
};

const clearAcknowledgedWarningProviderIds = (workspaceId: string): void => {
  try {
    window.sessionStorage.removeItem(warningAcknowledgementStorageKey(workspaceId));
  } catch {
    // Ignore storage failures and let the next navigation clear the acknowledgement.
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

  const installableProviderIds = flagged
    .filter((provider) => provider.installSupported)
    .map((provider) => provider.providerId);

  return {
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
  const flaggedProviderIds = useMemo(
    () => warning?.providers.map((provider) => provider.providerId) ?? [],
    [warning],
  );
  const [acknowledgedProviderIds, setAcknowledgedProviderIds] = useState<string[]>(() =>
    readAcknowledgedWarningProviderIds(workspaceId));
  const acknowledgedProviderIdSet = useMemo(
    () => new Set(acknowledgedProviderIds),
    [acknowledgedProviderIds],
  );

  useEffect(() => {
    setAcknowledgedProviderIds(readAcknowledgedWarningProviderIds(workspaceId));
  }, [workspaceId]);

  useEffect(() => {
    if (warning) return;
    clearAcknowledgedWarningProviderIds(workspaceId);
    setAcknowledgedProviderIds((current) => (current.length > 0 ? [] : current));
  }, [workspaceId, warning]);

  const warningAcknowledged =
    flaggedProviderIds.length > 0
    && flaggedProviderIds.every((providerId) => acknowledgedProviderIdSet.has(providerId));

  if (!warning || warningAcknowledged) return null;

  const acknowledgeWarning = () => {
    const nextAcknowledgedProviderIds = normalizeProviderIds([
      ...acknowledgedProviderIds,
      ...flaggedProviderIds,
    ]);
    persistAcknowledgedWarningProviderIds(workspaceId, nextAcknowledgedProviderIds);
    setAcknowledgedProviderIds(nextAcknowledgedProviderIds);
  };

  const dismiss = () => {
    acknowledgeWarning();
  };

  const handleOpenSettings = () => {
    dismiss();
    onOpenSettings();
  };

  const handleUpdateAll = async () => {
    acknowledgeWarning();
    try {
      await onUpdateProviders(warning.installableProviderIds);
    } catch {
      // The workbench already surfaces install errors independently.
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
