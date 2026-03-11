import {
  deleteProviderHarnessEndpoint,
  refreshProviderHarnessEndpointModels,
  selectProviderHarnessSource,
  upsertProviderHarnessEndpoint,
  verifyProviderForWorkspace,
  type HarnessApiShape,
  type HarnessEndpointRecord,
  type HarnessProviderSourceConfig,
  type HarnessSourceKind,
} from "../api/client";
import {
  getProvidersBootstrapSnapshotForScope,
  invalidateHostProvidersBootstrap,
  invalidateProvidersBootstrap,
  refreshProvidersBootstrapForScope,
  updateProvidersBootstrapForScope,
} from "./providersBootstrapStore";
import type { OwnerScope } from "./scopeIdentity";

export type ResolveUpsertedEndpointArgs = {
  requestedEndpointId: string | null;
  previousEndpointIds: Set<string>;
  nextEndpoints: HarnessEndpointRecord[];
  name: string;
  normalizedBase: string | null;
  authType?: string | null;
  geminiAuthType?: "gemini_api_key" | "vertex_ai" | null;
};

export type ProviderSourceSelection = {
  sourceKind: HarnessSourceKind;
  endpointId: string | null;
};

export type SelectSubscriptionSourceIfSupportedParams = {
  ownerScope: OwnerScope;
  providerId: string;
  supportsEndpointConfig: boolean;
  onEndpointUnsupported?: () => void;
};

export type SubmitProviderEndpointAuthParams = {
  ownerScope: OwnerScope;
  providerId: string;
  requestedEndpointId: string | null;
  name: string;
  baseUrl: string | null;
  apiShape: HarnessApiShape | null;
  authType: string | null;
  apiKey: string | null;
  serviceAccountJson: string | null;
  projectId: string | null;
  location: string | null;
  manualModelIds: string[];
  previousSelection: ProviderSourceSelection;
  isStale?: () => boolean;
};

export type SubmitProviderEndpointAuthResult =
  | {
    status: "applied";
    selectedEndpointId: string | null;
  }
  | {
    status: "stale";
    selectedEndpointId: string | null;
  }
  | {
    status: "rolled_back";
    selectedEndpointId: string | null;
    message: string | null;
  }
  | {
    status: "rollback_failed";
    selectedEndpointId: string | null;
    message: string | null;
    rollbackError: unknown;
  };

const UNSUPPORTED_HARNESS_ENDPOINTS_MESSAGE = "provider does not support harness endpoints";

const toErrorMessage = (error: unknown): string => {
  if (error instanceof Error) {
    return error.message;
  }
  return String(error ?? "");
};

const trimMessage = (message: string | null | undefined): string | null => {
  if (typeof message !== "string") return null;
  const trimmed = message.trim();
  return trimmed ? trimmed : null;
};

const invalidateBootstrapForOwner = (ownerScope: OwnerScope): void => {
  if (ownerScope.kind === "workspace") {
    invalidateProvidersBootstrap(ownerScope.workspaceId);
    return;
  }
  invalidateHostProvidersBootstrap();
};

const patchProviderHarnessConfig = (
  ownerScope: OwnerScope,
  providerId: string,
  nextConfig: HarnessProviderSourceConfig,
): HarnessProviderSourceConfig => {
  const next = updateProvidersBootstrapForScope(ownerScope, (current) => ({
    ...current,
    provider_harness_config: {
      ...current.provider_harness_config,
      [providerId]: nextConfig,
    },
  }));
  return next.provider_harness_config[providerId] ?? nextConfig;
};

const setSubscriptionSourceFallback = (
  ownerScope: OwnerScope,
  providerId: string,
): HarnessProviderSourceConfig => {
  const currentConfig = getProvidersBootstrapSnapshotForScope(ownerScope).provider_harness_config[providerId];
  return patchProviderHarnessConfig(ownerScope, providerId, {
    provider_id: providerId,
    selected_source_kind: "subscription",
    selected_endpoint_id: null,
    endpoints: currentConfig?.endpoints ?? [],
  });
};

const refreshBootstrapAfterMutation = async (ownerScope: OwnerScope): Promise<void> => {
  invalidateBootstrapForOwner(ownerScope);
  await refreshProvidersBootstrapForScope(ownerScope);
};

const applyProviderHarnessConfigMutation = async (
  ownerScope: OwnerScope,
  providerId: string,
  nextConfig: HarnessProviderSourceConfig,
): Promise<HarnessProviderSourceConfig> => {
  const patched = patchProviderHarnessConfig(ownerScope, providerId, nextConfig);
  await refreshBootstrapAfterMutation(ownerScope);
  return patched;
};

const selectProviderSourceInternal = async (
  ownerScope: OwnerScope,
  providerId: string,
  selection: ProviderSourceSelection,
): Promise<HarnessProviderSourceConfig> => {
  const next = await selectProviderHarnessSource(
    providerId,
    selection.sourceKind,
    selection.endpointId,
  );
  return applyProviderHarnessConfigMutation(ownerScope, providerId, next);
};

const rollbackPreviousSource = async (
  ownerScope: OwnerScope,
  providerId: string,
  previousSelection: ProviderSourceSelection,
): Promise<unknown | null> => {
  try {
    await selectProviderSourceInternal(ownerScope, providerId, previousSelection);
    return null;
  } catch (error) {
    return error;
  }
};

export const resolveUpsertedEndpoint = ({
  requestedEndpointId,
  previousEndpointIds,
  nextEndpoints,
  name,
  normalizedBase,
  authType,
  geminiAuthType,
}: ResolveUpsertedEndpointArgs): HarnessEndpointRecord | null => {
  const expectedAuthType = authType ?? geminiAuthType ?? null;

  if (requestedEndpointId) {
    return nextEndpoints.find((endpoint) => endpoint.id === requestedEndpointId) ?? null;
  }

  const newlyAdded = nextEndpoints.filter((endpoint) => !previousEndpointIds.has(endpoint.id));
  if (newlyAdded.length === 1) {
    return newlyAdded[0];
  }

  const reversedEndpoints = [...nextEndpoints].reverse();
  return reversedEndpoints.find(
    (endpoint) => endpoint.name === name
      && (endpoint.base_url ?? null) === normalizedBase
      && (expectedAuthType === null || endpoint.auth_type === expectedAuthType),
  )
    ?? nextEndpoints[nextEndpoints.length - 1]
    ?? null;
};

export const deleteProviderEndpoint = async (
  ownerScope: OwnerScope,
  providerId: string,
  endpointId: string,
): Promise<void> => {
  const next = await deleteProviderHarnessEndpoint(providerId, endpointId);
  await applyProviderHarnessConfigMutation(ownerScope, providerId, next);
};

export const refreshProviderEndpointModels = async (
  ownerScope: OwnerScope,
  providerId: string,
  endpointId: string,
): Promise<void> => {
  const next = await refreshProviderHarnessEndpointModels(providerId, endpointId);
  await applyProviderHarnessConfigMutation(ownerScope, providerId, next);
};

export const selectProviderSource = async (
  ownerScope: OwnerScope,
  providerId: string,
  selection: ProviderSourceSelection,
): Promise<void> => {
  await selectProviderSourceInternal(ownerScope, providerId, selection);
};

export const selectSubscriptionSourceIfSupported = async ({
  ownerScope,
  providerId,
  supportsEndpointConfig,
  onEndpointUnsupported,
}: SelectSubscriptionSourceIfSupportedParams): Promise<"selected" | "skipped" | "unsupported"> => {
  if (!supportsEndpointConfig) return "skipped";
  try {
    await selectProviderSourceInternal(ownerScope, providerId, {
      sourceKind: "subscription",
      endpointId: null,
    });
    return "selected";
  } catch (error) {
    if (toErrorMessage(error).includes(UNSUPPORTED_HARNESS_ENDPOINTS_MESSAGE)) {
      setSubscriptionSourceFallback(ownerScope, providerId);
      onEndpointUnsupported?.();
      return "unsupported";
    }
    throw error;
  }
};

export const submitProviderEndpointAuth = async ({
  ownerScope,
  providerId,
  requestedEndpointId,
  name,
  baseUrl,
  apiShape,
  authType,
  apiKey,
  serviceAccountJson,
  projectId,
  location,
  manualModelIds,
  previousSelection,
  isStale,
}: SubmitProviderEndpointAuthParams): Promise<SubmitProviderEndpointAuthResult> => {
  const stale = isStale ?? (() => false);
  const previousEndpointIds = new Set(
    (
      getProvidersBootstrapSnapshotForScope(ownerScope).provider_harness_config[providerId]?.endpoints
      ?? []
    ).map((endpoint) => endpoint.id),
  );
  const next = await upsertProviderHarnessEndpoint(providerId, {
    endpoint_id: requestedEndpointId,
    name,
    base_url: baseUrl,
    api_shape: apiShape,
    auth_type: authType,
    api_key: apiKey,
    service_account_json: serviceAccountJson,
    project_id: projectId,
    location,
    manual_model_ids: manualModelIds,
  });

  const upsertedEndpoint = resolveUpsertedEndpoint({
    requestedEndpointId,
    previousEndpointIds,
    nextEndpoints: next.endpoints,
    name,
    normalizedBase: baseUrl,
    authType,
  });
  const selectedEndpointId = upsertedEndpoint?.id ?? next.selected_endpoint_id ?? requestedEndpointId ?? null;

  if (stale()) {
    return {
      status: "stale",
      selectedEndpointId,
    };
  }

  await selectProviderSourceInternal(ownerScope, providerId, {
    sourceKind: "endpoint",
    endpointId: selectedEndpointId,
  });

  if (ownerScope.kind === "workspace") {
    const verify = await verifyProviderForWorkspace(ownerScope.workspaceId, providerId);
    if (stale()) {
      await rollbackPreviousSource(ownerScope, providerId, previousSelection);
      return {
        status: "stale",
        selectedEndpointId,
      };
    }
    if (verify.status !== "ok") {
      const rollbackError = await rollbackPreviousSource(ownerScope, providerId, previousSelection);
      const message = trimMessage(verify.message)
        ?? `Endpoint verification failed for ${providerId} (${verify.status}).`;
      if (rollbackError) {
        return {
          status: "rollback_failed",
          selectedEndpointId,
          message,
          rollbackError,
        };
      }
      return {
        status: "rolled_back",
        selectedEndpointId,
        message,
      };
    }
  } else if (stale()) {
    await rollbackPreviousSource(ownerScope, providerId, previousSelection);
    return {
      status: "stale",
      selectedEndpointId,
    };
  }

  return {
    status: "applied",
    selectedEndpointId,
  };
};
