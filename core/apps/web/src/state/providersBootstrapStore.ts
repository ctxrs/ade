import { getProvidersBootstrap, type ProvidersBootstrapResponse } from "../api/client";

type ProvidersBootstrapEntry = {
  data?: ProvidersBootstrapResponse;
  inFlight?: Promise<ProvidersBootstrapResponse>;
};

const providersBootstrapByWorkspace = new Map<string, ProvidersBootstrapEntry>();

function getOrCreateEntry(workspaceId: string): ProvidersBootstrapEntry {
  let entry = providersBootstrapByWorkspace.get(workspaceId);
  if (!entry) {
    entry = {};
    providersBootstrapByWorkspace.set(workspaceId, entry);
  }
  return entry;
}

function loadFresh(workspaceId: string, entry: ProvidersBootstrapEntry): Promise<ProvidersBootstrapResponse> {
  const request = getProvidersBootstrap(workspaceId)
    .then((next) => {
      entry.data = next;
      return next;
    })
    .finally(() => {
      if (entry.inFlight === request) {
        entry.inFlight = undefined;
      }
    });
  entry.inFlight = request;
  return request;
}

export function getCachedProvidersBootstrap(workspaceId: string): ProvidersBootstrapResponse | undefined {
  return providersBootstrapByWorkspace.get(workspaceId)?.data;
}

export function hasCachedProvidersBootstrap(workspaceId: string): boolean {
  return providersBootstrapByWorkspace.get(workspaceId)?.data !== undefined;
}

export async function loadProvidersBootstrap(workspaceId: string): Promise<ProvidersBootstrapResponse> {
  if (!workspaceId) {
    throw new Error("workspaceId is required");
  }
  const entry = getOrCreateEntry(workspaceId);
  if (entry.data) {
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
  providersBootstrapByWorkspace.delete(workspaceId);
}
