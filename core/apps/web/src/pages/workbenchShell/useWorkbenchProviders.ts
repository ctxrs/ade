import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type Dispatch,
  type SetStateAction,
} from "react";
import {
  type ProviderOptions,
} from "../../api/client";
import type { DraftHarness } from "../../components/WorkbenchComposer";
import { providerDetailFlag } from "../../utils/boolish";
import {
  resolveProviderOptionsUpdate,
  shouldHydrateProviderModels,
  useProviderOnboardingCoordinator,
  type ProviderAuthSummaryTrigger,
} from "../../state/providerOnboardingCoordinator";

type UseWorkbenchProvidersArgs = {
  workspaceId: string;
  setDraftHarness: Dispatch<SetStateAction<DraftHarness | null>>;
  onStartError: (message: string | null) => void;
};

export { resolveProviderOptionsUpdate, shouldHydrateProviderModels } from "../../state/providerOnboardingCoordinator";

const toErrorMessage = (error: unknown): string => {
  if (error instanceof Error) return error.message;
  return String(error);
};

export function useWorkbenchProviders({
  workspaceId,
  setDraftHarness,
  onStartError,
}: UseWorkbenchProvidersArgs) {
  const [installAllBusy, setInstallAllBusy] = useState(false);
  const onboarding = useProviderOnboardingCoordinator({
    workspaceId,
  });

  const providers = onboarding.bootstrap.providers;
  const providerOptions = onboarding.bootstrap.provider_options;
  const providersById = onboarding.providersById;
  const providerInstallsById = onboarding.installsById;

  const defaultProviderId = useMemo(() => {
    const installed = providers
      .filter((provider) => provider.installed && provider.health === "ok" && !providerDetailFlag(provider.details, "ui_hidden"))
      .map((provider) => provider.provider_id);
    if (installed.includes("codex")) return "codex";
    if (installed.includes("claude-crp")) return "claude-crp";
    if (installed.includes("gemini")) return "gemini";
    if (installed.includes("qwen")) return "qwen";
    if (installed.includes("opencode")) return "opencode";
    if (installed.includes("mistral")) return "mistral";
    if (installed.includes("goose")) return "goose";
    if (installed.includes("kimi")) return "kimi";
    if (installed.includes("auggie")) return "auggie";
    return installed[0] ?? "codex";
  }, [providers]);

  useEffect(() => {
    if (!providers.length) return;
    const codexInstalled =
      providersById.codex?.installed === true &&
      providersById.codex?.health === "ok" &&
      !providerDetailFlag(providersById.codex?.details, "ui_hidden");
    if (codexInstalled || defaultProviderId === "codex") return;
    setDraftHarness((prev) => {
      if (!prev) return prev;
      const isDefault = prev.providerId === "codex" && prev.modelId.trim().length === 0;
      if (!isDefault) return prev;
      return { ...prev, providerId: defaultProviderId };
    });
  }, [defaultProviderId, providers.length, providersById, setDraftHarness]);

  const installProviderFromMenu = useCallback(
    async (providerId: string) => {
      onStartError(null);
      try {
        await onboarding.startProviderInstall(providerId);
      } catch (error: unknown) {
        onStartError(toErrorMessage(error));
      }
    },
    [onStartError, onboarding],
  );

  const installAllProvidersFromMenu = useCallback(async () => {
    onStartError(null);
    setInstallAllBusy(true);
    try {
      await onboarding.startAllProviderInstalls();
    } catch (error: unknown) {
      onStartError(toErrorMessage(error));
    } finally {
      setInstallAllBusy(false);
    }
  }, [onStartError, onboarding]);

  const cancelProviderInstallFromMenu = useCallback(
    async (providerId: string) => {
      onStartError(null);
      try {
        await onboarding.cancelProviderInstall(providerId);
      } catch (error: unknown) {
        onStartError(toErrorMessage(error));
      }
    },
    [onStartError, onboarding],
  );

  const ensureProviderAuthSummary = useCallback(
    async (
      providerId: string,
      opts?: { force?: boolean; trigger?: ProviderAuthSummaryTrigger },
    ): Promise<ProviderOptions | undefined> => {
      return onboarding.ensureProviderAuthSummary(providerId, opts);
    },
    [onboarding],
  );

  return {
    providersById,
    defaultProviderId,
    providerInstallsById,
    providerOptions,
    installAllBusy,
    installProviderFromMenu,
    cancelProviderInstallFromMenu,
    installAllProvidersFromMenu,
    ensureProviderAuthSummary,
  };
}
