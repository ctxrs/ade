import { useCallback, useEffect, useMemo, useSyncExternalStore } from "react";
import {
  cancelInstall,
  getProviderOptions,
  installAllProviders,
  installProvider,
  type InstallInfo,
  type InstallStartResponse,
  type InstallTarget,
  type ProviderOptions,
  type ProviderStatus,
  type ProvidersBootstrapResponse,
} from "../api/client";
import { subscribeDaemonConnection } from "../api/daemonConnection";
import { computeInstallPct, parseInstallTarget } from "../utils/providerInstallUi";
import { isReadyVisibleHarnessProviderStatus } from "../utils/providerInventory";
import {
  EMPTY_PROVIDERS_BOOTSTRAP,
  loadHostProvidersBootstrap,
  loadProvidersBootstrap,
  refreshHostProvidersBootstrap,
  refreshProvidersBootstrap,
  resolveProviderOptionsUpdate,
  resolveProviderOptionsUpdateForScope,
  getProvidersBootstrapSnapshotForScope,
  loadProvidersBootstrapForScope,
  refreshProvidersBootstrapForScope,
  subscribeProvidersBootstrapForScope,
  updateProvidersBootstrapForScope,
} from "./providersBootstrapStore";
import {
  getProviderInstallProgressSnapshotForScope,
  resolveProviderInstallProgressSession,
  subscribeProviderInstallProgressForScope,
  upsertProviderInstallProgressForScope,
  type ProviderInstallProgressSnapshot,
} from "./providerInstallProgressStore";
import { observeInstall } from "./installProgressMonitor";
import { providerDetailFlag } from "../utils/boolish";
import {
  createMissingProviderOwnerScopeError,
  getProviderOwnerScope,
  getProviderOwnerScopeKeyOrNull,
  getProviderOwnerScopeOrNull,
} from "./providerScopeAdapters";
import {
  createProviderAuthScopeFromOptions,
  serializeOwnerScope,
  serializeProviderAuthScope,
  type OwnerScope,
  type WorkspaceOwnerScope,
} from "./scopeIdentity";
import {
  hasFailedProviderModelProbe,
  hasProviderModels,
  isFinalProviderModelCatalog,
  isEndpointProviderSourceSelected,
  isPinnedSubscriptionBootstrapCatalog,
} from "../utils/providerModelCatalog";
const MODEL_DISCOVERY_PROVIDER_IDS = new Set(["codex", "claude-crp", "copilot"]);

export { resolveProviderOptionsUpdate } from "./providersBootstrapStore";

export type ProviderAuthSummaryTrigger = "passive" | "explicit";

export type ProviderOnboardingInstallState = {
  installId: string;
  state: InstallInfo["state"];
  pct: number | null;
  target?: InstallTarget;
  errorCode?: InstallInfo["error_code"];
  error?: string;
};

export type ProviderOnboardingSnapshot = {
  bootstrap: ProvidersBootstrapResponse;
  providersById: Record<string, ProviderStatus>;
  installsById: Record<string, ProviderOnboardingInstallState>;
};

const EMPTY_PROVIDERS_BY_ID: Record<string, ProviderStatus> = {};
const EMPTY_INSTALLS_BY_ID: Record<string, ProviderOnboardingInstallState> = {};

const EMPTY_PROVIDER_ONBOARDING_SNAPSHOT: ProviderOnboardingSnapshot = Object.freeze({
  bootstrap: EMPTY_PROVIDERS_BOOTSTRAP,
  providersById: EMPTY_PROVIDERS_BY_ID,
  installsById: EMPTY_INSTALLS_BY_ID,
});

type ProviderOnboardingListener = () => void;

type ProviderOnboardingEntry = {
  scopeKey: string;
  ownerScope: OwnerScope;
  workspaceOwnerScope: WorkspaceOwnerScope | null;
  workspaceId: string | null;
  refCount: number;
  disposed: boolean;
  listeners: Set<ProviderOnboardingListener>;
  snapshot: ProviderOnboardingSnapshot;
  bootstrapUnsubscribe?: () => void;
  installProgressUnsubscribe?: () => void;
  installObserversByProviderId: Record<string, () => void>;
  previousInstallsById: Record<string, ProviderOnboardingInstallState>;
  postInstallHandledIds: Set<string>;
  postInstallInFlightProviderIds: Set<string>;
  providerAuthSummaryInFlightByKey: Record<string, Promise<ProviderOptions | undefined>>;
};

const providerOnboardingByScope = new Map<string, ProviderOnboardingEntry>();
const foregroundRefreshScopeKeys = new Set<string>();

let foregroundRefreshListenersInstalled = false;

const sameProviderInstallState = (
  lhs: ProviderOnboardingInstallState | undefined,
  rhs: ProviderOnboardingInstallState | undefined,
): boolean => {
  if (!lhs && !rhs) return true;
  if (!lhs || !rhs) return false;
  return lhs.installId === rhs.installId
    && lhs.state === rhs.state
    && lhs.pct === rhs.pct
    && lhs.target === rhs.target
    && lhs.errorCode === rhs.errorCode
    && lhs.error === rhs.error;
};

const sameProviderInstallStateMap = (
  lhs: Record<string, ProviderOnboardingInstallState>,
  rhs: Record<string, ProviderOnboardingInstallState>,
): boolean => {
  const lhsKeys = Object.keys(lhs);
  const rhsKeys = Object.keys(rhs);
  if (lhsKeys.length !== rhsKeys.length) return false;
  for (const key of lhsKeys) {
    if (!sameProviderInstallState(lhs[key], rhs[key])) {
      return false;
    }
  }
  return true;
};

const getProviderAccountIdentityById = (
  bootstrap: ProvidersBootstrapResponse,
): Record<string, string | null | undefined> => ({
  codex: bootstrap.codex_accounts.active_account_id,
  "claude-crp": bootstrap.claude_accounts.active_account_id,
  gemini: bootstrap.gemini_accounts.active_account_id,
  qwen: bootstrap.qwen_accounts.active_account_id,
  kimi: bootstrap.kimi_accounts.active_account_id,
  mistral: bootstrap.mistral_accounts.active_account_id,
  copilot: bootstrap.copilot_accounts.active_account_id,
  cursor: bootstrap.cursor_accounts.active_account_id,
  amp: bootstrap.amp_accounts.active_account_id,
  auggie: bootstrap.auggie_accounts?.active_account_id,
});

const withProviderAccountIdentity = (
  providerId: string,
  options: ProviderOptions | undefined,
  accountIdentityById: Record<string, string | null | undefined>,
): ProviderOptions | undefined => {
  if (!options) return options;
  const accountIdentity = accountIdentityById[providerId] ?? null;
  if (options.account_identity === accountIdentity) return options;
  return {
    ...options,
    account_identity: accountIdentity,
  };
};

const mergeProviderOptionsMap = (
  previous: Record<string, ProviderOptions>,
  next: Record<string, ProviderOptions>,
  workspaceOwnerScope: WorkspaceOwnerScope | null,
): Record<string, ProviderOptions> => {
  const merged = Object.fromEntries(
    Object.entries(next).map(([providerId, options]) => {
      const resolved = resolveProviderOptionsUpdateForScope(
        workspaceOwnerScope,
        previous[providerId],
        options,
      ) ?? options;
      return [providerId, resolved] as const;
    }),
  ) as Record<string, ProviderOptions>;

  const previousKeys = Object.keys(previous);
  const mergedKeys = Object.keys(merged);
  if (
    previousKeys.length === mergedKeys.length
    && mergedKeys.every((providerId) => previous[providerId] === merged[providerId])
  ) {
    return previous;
  }
  return merged;
};

const toProvidersById = (
  providers: ProviderStatus[],
): Record<string, ProviderStatus> => Object.fromEntries(
  providers.map((provider) => [provider.provider_id, provider]),
);

const withScopedProviderAccountIdentity = (
  scoped: ProviderOptions | undefined,
  next: ProviderOptions | undefined,
): ProviderOptions | undefined => {
  if (!next || !scoped) return next;
  const accountIdentity = scoped.account_identity ?? null;
  if (next.account_identity === accountIdentity) return next;
  return {
    ...next,
    account_identity: accountIdentity,
  };
};

const providerInstallTargetForProvider = (
  provider: ProviderStatus | undefined,
): InstallTarget | undefined => parseInstallTarget(provider?.details?.install_target);

const toProviderInstallState = (
  session: NonNullable<ReturnType<typeof resolveProviderInstallProgressSession>>,
): ProviderOnboardingInstallState => ({
  installId: session.installId,
  state: session.state,
  pct: session.pct,
  target: session.target,
  errorCode: session.errorCode,
  error: session.error,
});

const providerInstallsFromSnapshot = (
  snapshot: ProviderInstallProgressSnapshot,
  providersById: Record<string, ProviderStatus>,
): Record<string, ProviderOnboardingInstallState> =>
  Object.fromEntries(
    Array.from(new Set([...Object.keys(snapshot), ...Object.keys(providersById)]))
      .map((providerId) => {
        const session = resolveProviderInstallProgressSession(
          snapshot,
          providerId,
          providerInstallTargetForProvider(providersById[providerId]),
        );
        return session ? ([providerId, toProviderInstallState(session)] as const) : null;
      })
      .filter((entry): entry is readonly [string, ProviderOnboardingInstallState] => entry !== null),
  );

const shouldInstallForegroundRefreshListeners = (): boolean =>
  foregroundRefreshScopeKeys.size > 0
  && typeof window !== "undefined"
  && typeof document !== "undefined";

const refreshForegroundScopes = (): void => {
  for (const scopeKey of foregroundRefreshScopeKeys) {
    const entry = providerOnboardingByScope.get(scopeKey);
    if (!entry || entry.disposed || entry.refCount <= 0) continue;
    void refreshProvidersBootstrapForScope(entry.ownerScope).catch(() => {});
  }
};

const onVisibilityChange = (): void => {
  if (document.visibilityState === "visible") {
    refreshForegroundScopes();
  }
};

const syncForegroundRefreshListeners = (): void => {
  const shouldInstall = shouldInstallForegroundRefreshListeners();
  if (shouldInstall && !foregroundRefreshListenersInstalled) {
    window.addEventListener("focus", refreshForegroundScopes);
    window.addEventListener("online", refreshForegroundScopes);
    document.addEventListener("visibilitychange", onVisibilityChange);
    foregroundRefreshListenersInstalled = true;
    return;
  }
  if (!shouldInstall && foregroundRefreshListenersInstalled) {
    window.removeEventListener("focus", refreshForegroundScopes);
    window.removeEventListener("online", refreshForegroundScopes);
    document.removeEventListener("visibilitychange", onVisibilityChange);
    foregroundRefreshListenersInstalled = false;
  }
};

const emit = (entry: ProviderOnboardingEntry): void => {
  for (const listener of entry.listeners) {
    listener();
  }
};

const workspaceOwnerScopeFromOwner = (
  ownerScope: OwnerScope,
): WorkspaceOwnerScope | null => ownerScope.kind === "workspace" ? ownerScope : null;

const buildSnapshot = (ownerScope: OwnerScope): ProviderOnboardingSnapshot => {
  const bootstrap = getProvidersBootstrapSnapshotForScope(ownerScope);
  const providersById = toProvidersById(bootstrap.providers);
  const installsById = providerInstallsFromSnapshot(
    getProviderInstallProgressSnapshotForScope(ownerScope),
    providersById,
  );
  return {
    bootstrap,
    providersById,
    installsById,
  };
};

const getOrCreateEntry = (ownerScope: OwnerScope): ProviderOnboardingEntry => {
  const scopeKey = serializeOwnerScope(ownerScope);
  const existing = providerOnboardingByScope.get(scopeKey);
  if (existing) return existing;

  const workspaceOwnerScope = workspaceOwnerScopeFromOwner(ownerScope);
  const entry: ProviderOnboardingEntry = {
    scopeKey,
    ownerScope,
    workspaceOwnerScope,
    workspaceId: workspaceOwnerScope?.workspaceId ?? null,
    refCount: 0,
    disposed: false,
    listeners: new Set(),
    snapshot: buildSnapshot(ownerScope),
    installObserversByProviderId: {},
    previousInstallsById: {},
    postInstallHandledIds: new Set(),
    postInstallInFlightProviderIds: new Set(),
    providerAuthSummaryInFlightByKey: {},
  };
  providerOnboardingByScope.set(scopeKey, entry);
  return entry;
};

const maybeDeleteEntry = (entry: ProviderOnboardingEntry): void => {
  if (entry.refCount > 0 || entry.listeners.size > 0) return;
  providerOnboardingByScope.delete(entry.scopeKey);
};

const updateEntrySnapshot = (
  entry: ProviderOnboardingEntry,
  installProgressSnapshot?: ProviderInstallProgressSnapshot,
): void => {
  if (entry.disposed) return;

  const nextBootstrap = getProvidersBootstrapSnapshotForScope(entry.ownerScope);
  const nextProvidersById = toProvidersById(nextBootstrap.providers);
  const nextInstallsById = providerInstallsFromSnapshot(
    installProgressSnapshot ?? getProviderInstallProgressSnapshotForScope(entry.ownerScope),
    nextProvidersById,
  );

  if (
    entry.snapshot.bootstrap === nextBootstrap
    && sameProviderInstallStateMap(entry.snapshot.installsById, nextInstallsById)
  ) {
    return;
  }

  entry.snapshot = {
    bootstrap: nextBootstrap,
    providersById: nextProvidersById,
    installsById: nextInstallsById,
  };
  emit(entry);
};

const detachInstallObserver = (entry: ProviderOnboardingEntry, providerId: string): void => {
  const stop = entry.installObserversByProviderId[providerId];
  if (!stop) return;
  stop();
  delete entry.installObserversByProviderId[providerId];
};

const attachInstallObserver = (
  entry: ProviderOnboardingEntry,
  providerId: string,
  installId: string,
  initialTarget?: InstallTarget,
): void => {
  const existingInstallId = entry.snapshot.installsById[providerId]?.installId;
  if (existingInstallId === installId && entry.installObserversByProviderId[providerId]) {
    return;
  }

  detachInstallObserver(entry, providerId);
  entry.installObserversByProviderId[providerId] = observeInstall(installId, {
    ownerScope: entry.ownerScope,
    providerId,
    initialState: {
      state: "running",
      pct: entry.snapshot.installsById[providerId]?.pct ?? 0,
      target: initialTarget ?? entry.snapshot.installsById[providerId]?.target,
      errorCode: undefined,
      error: undefined,
    },
  });
};

const reconcileRunningInstalls = (entry: ProviderOnboardingEntry): void => {
  for (const provider of entry.snapshot.bootstrap.providers) {
    const installId = provider.details?.install_id;
    const running = providerDetailFlag(provider.details, "install_running");
    const tracked = entry.snapshot.installsById[provider.provider_id];
    if (
      running
      && installId
      && (!tracked || tracked.installId !== installId || tracked.state !== "running")
    ) {
      attachInstallObserver(
        entry,
        provider.provider_id,
        installId,
        providerInstallTargetForProvider(provider),
      );
    }
  }
};

const cleanupSucceededInstalls = (entry: ProviderOnboardingEntry): void => {
  if (entry.disposed) return;

  for (const [providerId, install] of Object.entries(entry.snapshot.installsById)) {
    const provider = entry.snapshot.providersById[providerId];
    const stillRunning = providerDetailFlag(provider?.details, "install_running");
    if (install.state === "succeeded" && isReadyVisibleHarnessProviderStatus(provider) && !stillRunning) {
      detachInstallObserver(entry, providerId);
    }
  }
};

export const shouldHydrateProviderModels = (
  providerId: string,
  options: ProviderOptions | undefined,
  trigger: ProviderAuthSummaryTrigger = "passive",
): boolean => {
  if (!MODEL_DISCOVERY_PROVIDER_IDS.has(providerId)) return false;
  if (!options) return false;
  if (isEndpointProviderSourceSelected(options)) return false;
  if (options.has_active_auth !== true) return false;
  if (trigger === "passive" && hasFailedProviderModelProbe(options)) return false;
  if (isPinnedSubscriptionBootstrapCatalog(options)) return true;
  if (hasProviderModels(options)) return !isFinalProviderModelCatalog(options);
  return true;
};

const ensureProviderAuthSummaryForEntry = async (
  entry: ProviderOnboardingEntry,
  providerId: string,
  opts?: { force?: boolean; trigger?: ProviderAuthSummaryTrigger },
): Promise<ProviderOptions | undefined> => {
  if (!entry.workspaceOwnerScope || !entry.workspaceId) return undefined;

  const ready = isReadyVisibleHarnessProviderStatus(entry.snapshot.providersById[providerId]);
  if (!ready) return undefined;

  const force = opts?.force ?? false;
  const trigger = opts?.trigger ?? (force ? "explicit" : "passive");
  const requestKey = serializeProviderAuthScope(
    createProviderAuthScopeFromOptions(
      entry.workspaceOwnerScope,
      providerId,
      entry.snapshot.bootstrap.provider_options[providerId],
    ),
  );
  const existing = entry.providerAuthSummaryInFlightByKey[requestKey];
  if (existing && !force) return existing;

  const cached = getProvidersBootstrapSnapshotForScope(entry.ownerScope).provider_options[providerId];
  if (!force && cached && !shouldHydrateProviderModels(providerId, cached, trigger)) {
    return cached;
  }

  const request = (
    force
      ? refreshProvidersBootstrapForScope(entry.ownerScope)
      : loadProvidersBootstrapForScope(entry.ownerScope)
  )
    .then(async (latestBootstrap) => {
      let next = resolveProviderOptionsUpdateForScope(
        entry.workspaceOwnerScope,
        cached,
        latestBootstrap.provider_options[providerId],
      );

      if (shouldHydrateProviderModels(providerId, next, trigger)) {
        try {
          const detailedResponse = await getProviderOptions(entry.workspaceId!, providerId);
          const detailed = withScopedProviderAccountIdentity(
            getProvidersBootstrapSnapshotForScope(entry.ownerScope).provider_options[providerId],
            detailedResponse,
          ) ?? detailedResponse;
          const updated = updateProvidersBootstrapForScope(entry.ownerScope, (current) => {
            const accountIdentityById = getProviderAccountIdentityById(current);
            const normalizedProviderOptions = Object.fromEntries(
              Object.entries(current.provider_options).map(([nextProviderId, options]) => [
                nextProviderId,
                withProviderAccountIdentity(nextProviderId, options, accountIdentityById),
              ]),
            ) as Record<string, ProviderOptions>;
            const resolved = resolveProviderOptionsUpdateForScope(
              entry.workspaceOwnerScope,
              normalizedProviderOptions[providerId],
              detailed,
            ) ?? detailed;
            if (normalizedProviderOptions[providerId] === resolved) {
              return current;
            }
            const nextProviderOptions = mergeProviderOptionsMap(normalizedProviderOptions, {
              ...normalizedProviderOptions,
              [providerId]: resolved,
            }, entry.workspaceOwnerScope);
            return {
              ...current,
              provider_options: nextProviderOptions,
            };
          });
          next = updated.provider_options[providerId];
        } catch {
          // Keep bootstrap options when probe hydration is unavailable.
        }
      }

      return next;
    })
    .finally(() => {
      if (entry.providerAuthSummaryInFlightByKey[requestKey] === request) {
        delete entry.providerAuthSummaryInFlightByKey[requestKey];
      }
    });

  entry.providerAuthSummaryInFlightByKey[requestKey] = request;
  return request;
};

const handleInstallTransitions = (entry: ProviderOnboardingEntry): void => {
  if (entry.disposed) return;

  let needsBootstrapRefresh = false;
  const completedProvidersNeedingAuthSummary: string[] = [];

  for (const [providerId, install] of Object.entries(entry.snapshot.installsById)) {
    const previous = entry.previousInstallsById[providerId];
    if (!install || install.state === "running") continue;
    if (previous?.installId === install.installId && previous.state === install.state) {
      continue;
    }

    needsBootstrapRefresh = true;
    if (
      entry.workspaceId
      && install.state === "succeeded"
      && !entry.postInstallHandledIds.has(install.installId)
    ) {
      entry.postInstallHandledIds.add(install.installId);
      completedProvidersNeedingAuthSummary.push(providerId);
    }
  }

  entry.previousInstallsById = entry.snapshot.installsById;

  if (!needsBootstrapRefresh) return;

  void refreshProvidersBootstrapForScope(entry.ownerScope)
    .then(() => {
      if (entry.disposed) return;
      for (const providerId of completedProvidersNeedingAuthSummary) {
        if (entry.postInstallInFlightProviderIds.has(providerId)) continue;
        entry.postInstallInFlightProviderIds.add(providerId);
        void ensureProviderAuthSummaryForEntry(entry, providerId)
          .catch(() => {})
          .finally(() => {
            entry.postInstallInFlightProviderIds.delete(providerId);
          });
      }
    })
    .catch(() => {});
};

const startEntry = (entry: ProviderOnboardingEntry): void => {
  if (entry.refCount <= 0) return;

  entry.disposed = false;
  foregroundRefreshScopeKeys.add(entry.scopeKey);
  syncForegroundRefreshListeners();

  entry.bootstrapUnsubscribe = subscribeProvidersBootstrapForScope(entry.ownerScope, () => {
    updateEntrySnapshot(entry);
    reconcileRunningInstalls(entry);
    cleanupSucceededInstalls(entry);
  });
  entry.installProgressUnsubscribe = subscribeProviderInstallProgressForScope(entry.ownerScope, (snapshot) => {
    updateEntrySnapshot(entry, snapshot);
    handleInstallTransitions(entry);
    cleanupSucceededInstalls(entry);
  });

  updateEntrySnapshot(entry);
  reconcileRunningInstalls(entry);
  handleInstallTransitions(entry);
  cleanupSucceededInstalls(entry);
};

const stopEntry = (entry: ProviderOnboardingEntry): void => {
  entry.disposed = true;
  entry.bootstrapUnsubscribe?.();
  entry.bootstrapUnsubscribe = undefined;
  entry.installProgressUnsubscribe?.();
  entry.installProgressUnsubscribe = undefined;

  for (const stop of Object.values(entry.installObserversByProviderId)) {
    stop();
  }
  entry.installObserversByProviderId = {};
  entry.providerAuthSummaryInFlightByKey = {};
  entry.postInstallInFlightProviderIds.clear();

  foregroundRefreshScopeKeys.delete(entry.scopeKey);
  syncForegroundRefreshListeners();
};

const retainEntry = (ownerScope: OwnerScope): (() => void) => {
  const entry = getOrCreateEntry(ownerScope);
  entry.refCount += 1;
  if (entry.refCount === 1) {
    startEntry(entry);
  }

  return () => {
    const current = providerOnboardingByScope.get(entry.scopeKey);
    if (!current) return;
    current.refCount = Math.max(0, current.refCount - 1);
    if (current.refCount === 0) {
      stopEntry(current);
    }
    maybeDeleteEntry(current);
  };
};

const getProviderOnboardingSnapshotForOwner = (
  ownerScope: OwnerScope,
): ProviderOnboardingSnapshot => getOrCreateEntry(ownerScope).snapshot;

const subscribeProviderOnboardingForOwner = (
  ownerScope: OwnerScope,
  listener: ProviderOnboardingListener,
): (() => void) => {
  const entry = getOrCreateEntry(ownerScope);
  entry.listeners.add(listener);
  return () => {
    entry.listeners.delete(listener);
    maybeDeleteEntry(entry);
  };
};

const ensureProviderAuthSummaryForOwner = (
  ownerScope: OwnerScope,
  providerId: string,
  opts?: { force?: boolean; trigger?: ProviderAuthSummaryTrigger },
): Promise<ProviderOptions | undefined> => ensureProviderAuthSummaryForEntry(
  getOrCreateEntry(ownerScope),
  providerId,
  opts,
);

export const getProviderOnboardingSnapshot = (
  workspaceId: string | null,
): ProviderOnboardingSnapshot => getProviderOnboardingSnapshotForOwner(getProviderOwnerScope(workspaceId));

export const subscribeProviderOnboarding = (
  workspaceId: string | null,
  listener: ProviderOnboardingListener,
): (() => void) => subscribeProviderOnboardingForOwner(getProviderOwnerScope(workspaceId), listener);

export const loadProviderOnboardingBootstrap = (
  workspaceId: string | null,
): Promise<ProvidersBootstrapResponse> =>
  workspaceId ? loadProvidersBootstrap(workspaceId) : loadHostProvidersBootstrap();

export const refreshProviderOnboardingBootstrap = (
  workspaceId: string | null,
): Promise<ProvidersBootstrapResponse> =>
  workspaceId ? refreshProvidersBootstrap(workspaceId) : refreshHostProvidersBootstrap();

export const ensureProviderAuthSummary = (
  workspaceId: string | null,
  providerId: string,
  opts?: { force?: boolean; trigger?: ProviderAuthSummaryTrigger },
): Promise<ProviderOptions | undefined> => ensureProviderAuthSummaryForOwner(
  getProviderOwnerScope(workspaceId),
  providerId,
  opts,
);

export const startProviderInstall = async (
  workspaceId: string | null,
  providerId: string,
): Promise<InstallStartResponse> => {
  const entry = getOrCreateEntry(getProviderOwnerScope(workspaceId));
  const target = providerInstallTargetForProvider(entry.snapshot.providersById[providerId]) ?? "host";
  const started = await installProvider(providerId, target);
  attachInstallObserver(entry, providerId, started.install_id, started.target);
  return started;
};

export const startAllProviderInstalls = async (
  workspaceId: string | null,
): Promise<InstallStartResponse[]> => {
  const entry = getOrCreateEntry(getProviderOwnerScope(workspaceId));
  const target = providerInstallTargetForProvider(
    entry.snapshot.bootstrap.providers.find((provider) => provider.details?.install_target),
  ) ?? "host";
  const started = await installAllProviders(target);
  for (const install of started) {
    attachInstallObserver(entry, install.provider_id, install.install_id, install.target);
  }
  return started;
};

export const cancelProviderOnboardingInstall = async (
  workspaceId: string | null,
  providerId: string,
): Promise<InstallInfo | undefined> => {
  const entry = getOrCreateEntry(getProviderOwnerScope(workspaceId));
  const install = entry.snapshot.installsById[providerId];
  const installId = install?.installId ?? entry.snapshot.providersById[providerId]?.details?.install_id;
  if (!installId) return undefined;

  const info = await cancelInstall(installId);
  upsertProviderInstallProgressForScope(entry.ownerScope, providerId, {
    installId,
    state: info.state,
    pct: computeInstallPct(info, install?.pct ?? null),
    target: info.target,
    errorCode: info.error_code,
    error: info.error,
  });
  return info;
};

export const resetProviderOnboardingCoordinatorForTests = (): void => {
  for (const entry of providerOnboardingByScope.values()) {
    stopEntry(entry);
  }
  providerOnboardingByScope.clear();
  foregroundRefreshScopeKeys.clear();
  syncForegroundRefreshListeners();
};

export const useProviderOnboardingCoordinator = ({
  workspaceId,
  enabled = true,
  onLoadError,
}: {
  workspaceId: string | null;
  enabled?: boolean;
  onLoadError?: (error: unknown) => void;
}) => {
  const ownerScopeKey = useSyncExternalStore(
    useCallback((listener) => subscribeDaemonConnection((_connection) => listener()), []),
    useCallback(() => getProviderOwnerScopeKeyOrNull(workspaceId), [workspaceId]),
    useCallback(() => getProviderOwnerScopeKeyOrNull(workspaceId), [workspaceId]),
  );
  const ownerScope = useMemo(
    () => getProviderOwnerScopeOrNull(workspaceId),
    [ownerScopeKey, workspaceId],
  );

  const snapshot = useSyncExternalStore(
    useCallback(
      (listener) => ownerScope ? subscribeProviderOnboardingForOwner(ownerScope, listener) : () => {},
      [ownerScope],
    ),
    useCallback(
      () => ownerScope ? getProviderOnboardingSnapshotForOwner(ownerScope) : EMPTY_PROVIDER_ONBOARDING_SNAPSHOT,
      [ownerScope],
    ),
    useCallback(
      () => ownerScope ? getProviderOnboardingSnapshotForOwner(ownerScope) : EMPTY_PROVIDER_ONBOARDING_SNAPSHOT,
      [ownerScope],
    ),
  );

  useEffect(() => {
    if (!enabled || !ownerScope) return;
    return retainEntry(ownerScope);
  }, [enabled, ownerScope]);

  useEffect(() => {
    if (!enabled || !ownerScope) return;
    loadProviderOnboardingBootstrap(workspaceId).catch((error) => {
      onLoadError?.(error);
    });
  }, [enabled, onLoadError, ownerScope, ownerScopeKey, workspaceId]);

  const loadBootstrap = useCallback(
    () => ownerScope ? loadProviderOnboardingBootstrap(workspaceId) : Promise.reject(createMissingProviderOwnerScopeError()),
    [ownerScope, ownerScopeKey, workspaceId],
  );
  const refreshBootstrap = useCallback(
    () => ownerScope ? refreshProviderOnboardingBootstrap(workspaceId) : Promise.reject(createMissingProviderOwnerScopeError()),
    [ownerScope, ownerScopeKey, workspaceId],
  );
  const ensureAuthSummary = useCallback(
    (providerId: string, opts?: { force?: boolean; trigger?: ProviderAuthSummaryTrigger }) =>
      ownerScope
        ? ensureProviderAuthSummaryForOwner(ownerScope, providerId, opts)
        : Promise.reject(createMissingProviderOwnerScopeError()),
    [ownerScope],
  );
  const onInstallProvider = useCallback(
    (providerId: string) => startProviderInstall(workspaceId, providerId),
    [workspaceId],
  );
  const onInstallAllProviders = useCallback(
    () => startAllProviderInstalls(workspaceId),
    [workspaceId],
  );
  const onCancelInstall = useCallback(
    (providerId: string) => cancelProviderOnboardingInstall(workspaceId, providerId),
    [workspaceId],
  );

  return useMemo(() => ({
    ...snapshot,
    loadBootstrap,
    refreshBootstrap,
    ensureProviderAuthSummary: ensureAuthSummary,
    startProviderInstall: onInstallProvider,
    startAllProviderInstalls: onInstallAllProviders,
    cancelProviderInstall: onCancelInstall,
  }), [
    ensureAuthSummary,
    loadBootstrap,
    onCancelInstall,
    onInstallAllProviders,
    onInstallProvider,
    refreshBootstrap,
    snapshot,
  ]);
};
