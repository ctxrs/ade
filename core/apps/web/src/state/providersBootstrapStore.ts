import {
  getProviderHarnessConfig,
  getProvidersBootstrap,
  listAmpAccounts,
  listClaudeAccounts,
  listCodexAccounts,
  listCopilotAccounts,
  listCursorAccounts,
  listGeminiAccounts,
  listKimiAccounts,
  listMistralAccounts,
  listProviders,
  listQwenAccounts,
  type ProviderOptions,
  type ProvidersBootstrapResponse,
} from "../api/client";
import {
  createProviderAuthScopeFromOptions,
  sameProviderAuthScope,
  serializeOwnerScope,
  type OwnerScope,
  type WorkspaceOwnerScope,
} from "./scopeIdentity";
import {
  getProviderHostOwnerScope,
  getProviderWorkspaceOwnerScope,
} from "./providerScopeAdapters";
import {
  hasFailedProviderModelProbe,
  hasProviderModels,
  isPinnedSubscriptionBootstrapCatalog,
} from "../utils/providerModelCatalog";

type Listener = () => void;

type ProvidersBootstrapEntry = {
  data?: ProvidersBootstrapResponse;
  inFlight?: Promise<ProvidersBootstrapResponse>;
  stale?: boolean;
  listeners: Set<Listener>;
};

type ProvidersBootstrapUpdater = (
  current: ProvidersBootstrapResponse,
) => ProvidersBootstrapResponse;

const EMPTY_ACCOUNTS = Object.freeze({
  active_account_id: null,
  accounts: [],
});

export const EMPTY_PROVIDERS_BOOTSTRAP: ProvidersBootstrapResponse = Object.freeze({
  providers: [],
  provider_options: {},
  provider_harness_config: {},
  codex_accounts: {
    ...EMPTY_ACCOUNTS,
    logins: [],
  },
  claude_accounts: EMPTY_ACCOUNTS,
  gemini_accounts: EMPTY_ACCOUNTS,
  qwen_accounts: EMPTY_ACCOUNTS,
  kimi_accounts: EMPTY_ACCOUNTS,
  mistral_accounts: EMPTY_ACCOUNTS,
  copilot_accounts: EMPTY_ACCOUNTS,
  cursor_accounts: EMPTY_ACCOUNTS,
  amp_accounts: EMPTY_ACCOUNTS,
});

const providersBootstrapByScope = new Map<string, ProvidersBootstrapEntry>();

type HostProvidersBootstrapSlices = Pick<
  ProvidersBootstrapResponse,
  | "providers"
  | "provider_harness_config"
  | "codex_accounts"
  | "claude_accounts"
  | "gemini_accounts"
  | "qwen_accounts"
  | "kimi_accounts"
  | "mistral_accounts"
  | "copilot_accounts"
  | "cursor_accounts"
  | "amp_accounts"
>;

const scopeKeyForOwner = (ownerScope: OwnerScope): string => serializeOwnerScope(ownerScope);

const matchesWorkspaceOwnerScope = (
  ownerScope: WorkspaceOwnerScope,
  options: ProviderOptions | undefined,
): boolean => options?.workspace_id === ownerScope.workspaceId;

const sameProviderOptionsAuthScope = (
  ownerScope: WorkspaceOwnerScope,
  lhs: ProviderOptions | undefined,
  rhs: ProviderOptions | undefined,
): boolean => {
  if (!lhs || !rhs) return false;
  if (!matchesWorkspaceOwnerScope(ownerScope, lhs) || !matchesWorkspaceOwnerScope(ownerScope, rhs)) {
    return false;
  }
  return sameProviderAuthScope(
    createProviderAuthScopeFromOptions(ownerScope, lhs.provider_id, lhs),
    createProviderAuthScopeFromOptions(ownerScope, rhs.provider_id, rhs),
  );
};

const sameProviderOptions = (
  lhs: ProviderOptions | undefined,
  rhs: ProviderOptions | undefined,
): boolean => JSON.stringify(lhs ?? null) === JSON.stringify(rhs ?? null);

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

export const resolveProviderOptionsUpdateForScope = (
  ownerScope: WorkspaceOwnerScope | null,
  previous: ProviderOptions | undefined,
  next: ProviderOptions | undefined,
): ProviderOptions | undefined => {
  if (!next) return previous === undefined ? previous : next;
  let resolved = next;
  if (
    ownerScope
    && previous
    && sameProviderOptionsAuthScope(ownerScope, previous, next)
    && next.has_active_auth === true
    && !hasProviderModels(next)
  ) {
    if (hasProviderModels(previous)) {
      resolved = { ...resolved, models: previous.models };
    }
    if (hasFailedProviderModelProbe(previous) && !hasFailedProviderModelProbe(resolved)) {
      resolved = {
        ...resolved,
        probe_ok: previous.probe_ok,
        probe_error: previous.probe_error ?? resolved.probe_error,
      };
    }
  }
  if (
    ownerScope
    && previous
    && sameProviderOptionsAuthScope(ownerScope, previous, next)
    && next.has_active_auth === true
    && isPinnedSubscriptionBootstrapCatalog(next)
    && hasProviderModels(previous)
    && !isPinnedSubscriptionBootstrapCatalog(previous)
  ) {
    resolved = { ...resolved, models: previous.models };
    if (hasFailedProviderModelProbe(previous) && !hasFailedProviderModelProbe(resolved)) {
      resolved = {
        ...resolved,
        probe_ok: previous.probe_ok,
        probe_error: previous.probe_error ?? resolved.probe_error,
      };
    }
  }
  return sameProviderOptions(previous, resolved) ? previous : resolved;
};

export const resolveProviderOptionsUpdate = (
  previous: ProviderOptions | undefined,
  next: ProviderOptions | undefined,
): ProviderOptions | undefined => {
  const workspaceId = next?.workspace_id ?? previous?.workspace_id;
  const ownerScope = workspaceId ? getProviderWorkspaceOwnerScope(workspaceId) : null;
  return resolveProviderOptionsUpdateForScope(ownerScope, previous, next);
};

const mergeProviderOptionsMap = (
  previous: Record<string, ProviderOptions>,
  next: Record<string, ProviderOptions>,
  ownerScope: WorkspaceOwnerScope | null,
): Record<string, ProviderOptions> => {
  const merged = Object.fromEntries(
    Object.entries(next).map(([providerId, options]) => {
      const resolved = resolveProviderOptionsUpdateForScope(
        ownerScope,
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

const normalizeProvidersBootstrap = (
  bootstrap: ProvidersBootstrapResponse,
  previous?: ProvidersBootstrapResponse,
  ownerScope?: OwnerScope,
): ProvidersBootstrapResponse => {
  const accountIdentityById = getProviderAccountIdentityById(bootstrap);
  const normalizedProviderOptions = Object.fromEntries(
    Object.entries(bootstrap.provider_options).map(([providerId, options]) => [
      providerId,
      withProviderAccountIdentity(providerId, options, accountIdentityById),
    ]),
  ) as Record<string, ProviderOptions>;
  return {
    ...bootstrap,
    provider_options: mergeProviderOptionsMap(
      previous?.provider_options ?? {},
      normalizedProviderOptions,
      ownerScope?.kind === "workspace" ? ownerScope : null,
    ),
  };
};

function getOrCreateEntry(scopeKey: string): ProvidersBootstrapEntry {
  let entry = providersBootstrapByScope.get(scopeKey);
  if (!entry) {
    entry = {
      listeners: new Set(),
    };
    providersBootstrapByScope.set(scopeKey, entry);
  }
  return entry;
}

function emit(entry: ProvidersBootstrapEntry): void {
  for (const listener of entry.listeners) {
    listener();
  }
}

function setEntryData(
  ownerScope: OwnerScope,
  entry: ProvidersBootstrapEntry,
  next: ProvidersBootstrapResponse,
): ProvidersBootstrapResponse {
  const scopeKey = scopeKeyForOwner(ownerScope);
  const normalized = normalizeProvidersBootstrap(next, entry.data, ownerScope);
  entry.data = normalized;
  entry.stale = false;
  providersBootstrapByScope.set(scopeKey, entry);
  emit(entry);
  return normalized;
}

function loadFresh(
  ownerScope: OwnerScope,
  entry: ProvidersBootstrapEntry,
  load: (current: ProvidersBootstrapResponse | undefined) => Promise<ProvidersBootstrapResponse>,
): Promise<ProvidersBootstrapResponse> {
  const request = load(entry.data)
    .then((next) => setEntryData(ownerScope, entry, next))
    .finally(() => {
      if (entry.inFlight === request) {
        entry.inFlight = undefined;
      }
    });
  entry.inFlight = request;
  entry.stale = false;
  return request;
}

async function loadHostBootstrapSlices(): Promise<HostProvidersBootstrapSlices> {
  const providers = await listProviders("host");
  const [
    codex_accounts,
    claude_accounts,
    gemini_accounts,
    qwen_accounts,
    kimi_accounts,
    mistral_accounts,
    copilot_accounts,
    cursor_accounts,
    amp_accounts,
    providerConfigs,
  ] = await Promise.all([
    listCodexAccounts(),
    listClaudeAccounts(),
    listGeminiAccounts(),
    listQwenAccounts(),
    listKimiAccounts(),
    listMistralAccounts(),
    listCopilotAccounts(),
    listCursorAccounts(),
    listAmpAccounts(),
    Promise.all(
      providers.map(async (provider) => (
        [provider.provider_id, await getProviderHarnessConfig(provider.provider_id)] as const
      )),
    ),
  ]);

  return {
    providers,
    provider_harness_config: Object.fromEntries(providerConfigs),
    codex_accounts,
    claude_accounts,
    gemini_accounts,
    qwen_accounts,
    kimi_accounts,
    mistral_accounts,
    copilot_accounts,
    cursor_accounts,
    amp_accounts,
  };
}

export function getCachedProvidersBootstrap(workspaceId: string): ProvidersBootstrapResponse | undefined {
  return getCachedProvidersBootstrapForScope(getProviderWorkspaceOwnerScope(workspaceId));
}

export function getProvidersBootstrapSnapshot(workspaceId: string): ProvidersBootstrapResponse {
  return getProvidersBootstrapSnapshotForScope(getProviderWorkspaceOwnerScope(workspaceId));
}

export function subscribeProvidersBootstrap(workspaceId: string, listener: Listener): () => void {
  if (!workspaceId) {
    return () => {};
  }
  return subscribeProvidersBootstrapForScope(getProviderWorkspaceOwnerScope(workspaceId), listener);
}

export function updateProvidersBootstrap(
  workspaceId: string,
  updater: ProvidersBootstrapUpdater,
): ProvidersBootstrapResponse {
  if (!workspaceId) {
    throw new Error("workspaceId is required");
  }
  return updateProvidersBootstrapForScope(getProviderWorkspaceOwnerScope(workspaceId), updater);
}

export function hasCachedProvidersBootstrap(workspaceId: string): boolean {
  return hasCachedProvidersBootstrapForScope(getProviderWorkspaceOwnerScope(workspaceId));
}

export async function loadProvidersBootstrap(workspaceId: string): Promise<ProvidersBootstrapResponse> {
  if (!workspaceId) {
    throw new Error("workspaceId is required");
  }
  return loadProvidersBootstrapForScope(getProviderWorkspaceOwnerScope(workspaceId));
}

export async function refreshProvidersBootstrap(workspaceId: string): Promise<ProvidersBootstrapResponse> {
  if (!workspaceId) {
    throw new Error("workspaceId is required");
  }
  return refreshProvidersBootstrapForScope(getProviderWorkspaceOwnerScope(workspaceId));
}

export function invalidateProvidersBootstrap(workspaceId: string): void {
  if (!workspaceId) return;
  invalidateProvidersBootstrapForScope(getProviderWorkspaceOwnerScope(workspaceId));
}

export function getCachedHostProvidersBootstrap(): ProvidersBootstrapResponse | undefined {
  return getCachedProvidersBootstrapForScope(getProviderHostOwnerScope());
}

export function getHostProvidersBootstrapSnapshot(): ProvidersBootstrapResponse {
  return getProvidersBootstrapSnapshotForScope(getProviderHostOwnerScope());
}

export function subscribeHostProvidersBootstrap(listener: Listener): () => void {
  return subscribeProvidersBootstrapForScope(getProviderHostOwnerScope(), listener);
}

export function updateHostProvidersBootstrap(
  updater: ProvidersBootstrapUpdater,
): ProvidersBootstrapResponse {
  return updateProvidersBootstrapForScope(getProviderHostOwnerScope(), updater);
}

export function hasCachedHostProvidersBootstrap(): boolean {
  return hasCachedProvidersBootstrapForScope(getProviderHostOwnerScope());
}

export async function loadHostProvidersBootstrap(): Promise<ProvidersBootstrapResponse> {
  return loadProvidersBootstrapForScope(getProviderHostOwnerScope());
}

export async function refreshHostProvidersBootstrap(): Promise<ProvidersBootstrapResponse> {
  return refreshProvidersBootstrapForScope(getProviderHostOwnerScope());
}

export function invalidateHostProvidersBootstrap(): void {
  invalidateProvidersBootstrapForScope(getProviderHostOwnerScope());
}

export function getCachedProvidersBootstrapForScope(
  ownerScope: OwnerScope,
): ProvidersBootstrapResponse | undefined {
  return providersBootstrapByScope.get(scopeKeyForOwner(ownerScope))?.data;
}

export function getProvidersBootstrapSnapshotForScope(ownerScope: OwnerScope): ProvidersBootstrapResponse {
  return getCachedProvidersBootstrapForScope(ownerScope) ?? EMPTY_PROVIDERS_BOOTSTRAP;
}

export function subscribeProvidersBootstrapForScope(
  ownerScope: OwnerScope,
  listener: Listener,
): () => void {
  const scopeKey = scopeKeyForOwner(ownerScope);
  const entry = getOrCreateEntry(scopeKey);
  entry.listeners.add(listener);
  return () => {
    entry.listeners.delete(listener);
    if (entry.listeners.size === 0 && !entry.inFlight && !entry.data) {
      providersBootstrapByScope.delete(scopeKey);
    }
  };
}

export function updateProvidersBootstrapForScope(
  ownerScope: OwnerScope,
  updater: ProvidersBootstrapUpdater,
): ProvidersBootstrapResponse {
  const entry = getOrCreateEntry(scopeKeyForOwner(ownerScope));
  const next = updater(entry.data ?? EMPTY_PROVIDERS_BOOTSTRAP);
  return setEntryData(ownerScope, entry, next);
}

export function hasCachedProvidersBootstrapForScope(ownerScope: OwnerScope): boolean {
  return getCachedProvidersBootstrapForScope(ownerScope) !== undefined;
}

export async function loadProvidersBootstrapForScope(ownerScope: OwnerScope): Promise<ProvidersBootstrapResponse> {
  const scopeKey = scopeKeyForOwner(ownerScope);
  const entry = getOrCreateEntry(scopeKey);
  if (entry.data && !entry.stale) {
    return entry.data;
  }
  if (entry.inFlight) {
    return entry.inFlight;
  }
  if (ownerScope.kind === "workspace") {
    return loadFresh(ownerScope, entry, () => getProvidersBootstrap(ownerScope.workspaceId));
  }
  return loadFresh(ownerScope, entry, async (current) => ({
    ...(current ?? EMPTY_PROVIDERS_BOOTSTRAP),
    ...(await loadHostBootstrapSlices()),
  }));
}

export async function refreshProvidersBootstrapForScope(
  ownerScope: OwnerScope,
): Promise<ProvidersBootstrapResponse> {
  const entry = getOrCreateEntry(scopeKeyForOwner(ownerScope));
  if (entry.inFlight) {
    return entry.inFlight;
  }
  if (ownerScope.kind === "workspace") {
    return loadFresh(ownerScope, entry, () => getProvidersBootstrap(ownerScope.workspaceId));
  }
  return loadFresh(ownerScope, entry, async (current) => ({
    ...(current ?? EMPTY_PROVIDERS_BOOTSTRAP),
    ...(await loadHostBootstrapSlices()),
  }));
}

export function invalidateProvidersBootstrapForScope(ownerScope: OwnerScope): void {
  const entry = providersBootstrapByScope.get(scopeKeyForOwner(ownerScope));
  if (!entry) return;
  entry.stale = true;
}
