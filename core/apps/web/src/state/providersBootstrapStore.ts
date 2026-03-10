import { getProvidersBootstrap, type ProviderOptions, type ProvidersBootstrapResponse } from "../api/client";

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

const providersBootstrapByWorkspace = new Map<string, ProvidersBootstrapEntry>();

const hasProviderModels = (options: ProviderOptions | undefined): boolean => {
  const raw = options?.models;
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return false;
  const record = raw as Record<string, unknown>;
  const current = record.currentModelId ?? record.current_model_id;
  if (typeof current === "string" && current.trim().length > 0) return true;
  const list = record.availableModels ?? record.available_models ?? record.models;
  return Array.isArray(list) && list.length > 0;
};

const hasFailedProviderModelProbe = (options: ProviderOptions | undefined): boolean => {
  if (!options) return false;
  if (options.probe_ok === false) return true;
  return typeof options.probe_error === "string" && options.probe_error.trim().length > 0;
};

const selectedEndpointScopeVersion = (options: ProviderOptions | undefined): string | null => {
  if (!options || options.source?.selected_source_kind !== "endpoint") return null;
  const endpointId = options.source.selected_endpoint_id;
  if (!endpointId) return null;
  const endpoint = options.source.endpoints.find((candidate) => candidate.id === endpointId);
  if (!endpoint) return endpointId;
  return [
    endpoint.id,
    endpoint.updated_at,
    endpoint.base_url ?? "",
    endpoint.has_api_key ? "1" : "0",
    endpoint.model_override ?? "",
  ].join(":");
};

const sameProviderOptionsScope = (
  lhs: ProviderOptions | undefined,
  rhs: ProviderOptions | undefined,
): boolean => {
  if (!lhs || !rhs) return false;
  return lhs.provider_id === rhs.provider_id
    && lhs.workspace_id === rhs.workspace_id
    && lhs.auth_mode === rhs.auth_mode
    && lhs.account_identity === rhs.account_identity
    && lhs.has_active_auth === rhs.has_active_auth
    && lhs.source?.selected_source_kind === rhs.source?.selected_source_kind
    && lhs.source?.selected_endpoint_id === rhs.source?.selected_endpoint_id
    && selectedEndpointScopeVersion(lhs) === selectedEndpointScopeVersion(rhs);
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

export const resolveProviderOptionsUpdate = (
  previous: ProviderOptions | undefined,
  next: ProviderOptions | undefined,
): ProviderOptions | undefined => {
  if (!next) return previous === undefined ? previous : next;
  let resolved = next;
  if (previous && sameProviderOptionsScope(previous, next) && next.has_active_auth === true && !hasProviderModels(next)) {
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
  return sameProviderOptions(previous, resolved) ? previous : resolved;
};

const mergeProviderOptionsMap = (
  previous: Record<string, ProviderOptions>,
  next: Record<string, ProviderOptions>,
): Record<string, ProviderOptions> => {
  const merged = Object.fromEntries(
    Object.entries(next).map(([providerId, options]) => {
      const resolved = resolveProviderOptionsUpdate(previous[providerId], options) ?? options;
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
    ),
  };
};

function getOrCreateEntry(workspaceId: string): ProvidersBootstrapEntry {
  let entry = providersBootstrapByWorkspace.get(workspaceId);
  if (!entry) {
    entry = {
      listeners: new Set(),
    };
    providersBootstrapByWorkspace.set(workspaceId, entry);
  }
  return entry;
}

function emit(entry: ProvidersBootstrapEntry): void {
  for (const listener of entry.listeners) {
    listener();
  }
}

function setEntryData(
  workspaceId: string,
  entry: ProvidersBootstrapEntry,
  next: ProvidersBootstrapResponse,
): ProvidersBootstrapResponse {
  const normalized = normalizeProvidersBootstrap(next, entry.data);
  entry.data = normalized;
  entry.stale = false;
  providersBootstrapByWorkspace.set(workspaceId, entry);
  emit(entry);
  return normalized;
}

function loadFresh(workspaceId: string, entry: ProvidersBootstrapEntry): Promise<ProvidersBootstrapResponse> {
  const request = getProvidersBootstrap(workspaceId)
    .then((next) => setEntryData(workspaceId, entry, next))
    .finally(() => {
      if (entry.inFlight === request) {
        entry.inFlight = undefined;
      }
    });
  entry.inFlight = request;
  entry.stale = false;
  return request;
}

export function getCachedProvidersBootstrap(workspaceId: string): ProvidersBootstrapResponse | undefined {
  return providersBootstrapByWorkspace.get(workspaceId)?.data;
}

export function getProvidersBootstrapSnapshot(workspaceId: string): ProvidersBootstrapResponse {
  return getCachedProvidersBootstrap(workspaceId) ?? EMPTY_PROVIDERS_BOOTSTRAP;
}

export function subscribeProvidersBootstrap(workspaceId: string, listener: Listener): () => void {
  if (!workspaceId) {
    return () => {};
  }
  const entry = getOrCreateEntry(workspaceId);
  entry.listeners.add(listener);
  return () => {
    entry.listeners.delete(listener);
    if (entry.listeners.size === 0 && !entry.inFlight && !entry.data) {
      providersBootstrapByWorkspace.delete(workspaceId);
    }
  };
}

export function updateProvidersBootstrap(
  workspaceId: string,
  updater: ProvidersBootstrapUpdater,
): ProvidersBootstrapResponse {
  if (!workspaceId) {
    throw new Error("workspaceId is required");
  }
  const entry = getOrCreateEntry(workspaceId);
  const next = updater(entry.data ?? EMPTY_PROVIDERS_BOOTSTRAP);
  return setEntryData(workspaceId, entry, next);
}

export function hasCachedProvidersBootstrap(workspaceId: string): boolean {
  return providersBootstrapByWorkspace.get(workspaceId)?.data !== undefined;
}

export async function loadProvidersBootstrap(workspaceId: string): Promise<ProvidersBootstrapResponse> {
  if (!workspaceId) {
    throw new Error("workspaceId is required");
  }
  const entry = getOrCreateEntry(workspaceId);
  if (entry.data && !entry.stale) {
    return entry.data;
  }
  if (entry.inFlight) {
    return entry.inFlight;
  }
  return loadFresh(workspaceId, entry);
}

export async function refreshProvidersBootstrap(workspaceId: string): Promise<ProvidersBootstrapResponse> {
  if (!workspaceId) {
    throw new Error("workspaceId is required");
  }
  const entry = getOrCreateEntry(workspaceId);
  if (entry.inFlight) {
    return entry.inFlight;
  }
  return loadFresh(workspaceId, entry);
}

export function invalidateProvidersBootstrap(workspaceId: string): void {
  if (!workspaceId) return;
  const entry = providersBootstrapByWorkspace.get(workspaceId);
  if (!entry) return;
  entry.stale = true;
}
