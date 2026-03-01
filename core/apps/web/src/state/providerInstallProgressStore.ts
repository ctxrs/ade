import type { InstallInfo } from "../api/client";

export type ProviderInstallProgressSession = {
  installId: string;
  state: InstallInfo["state"];
  pct: number | null;
  target?: InstallInfo["target"];
  errorCode?: InstallInfo["error_code"];
  error?: string;
  updatedAtMs: number;
};

export type ProviderInstallProgressSnapshot = Record<string, ProviderInstallProgressSession>;

type Listener = (snapshot: ProviderInstallProgressSnapshot) => void;

const installsByProviderId = new Map<string, ProviderInstallProgressSession>();
const listeners = new Set<Listener>();

function sameSession(
  lhs: ProviderInstallProgressSession | undefined,
  rhs: ProviderInstallProgressSession,
): boolean {
  if (!lhs) return false;
  return lhs.installId === rhs.installId
    && lhs.state === rhs.state
    && lhs.pct === rhs.pct
    && lhs.target === rhs.target
    && lhs.errorCode === rhs.errorCode
    && lhs.error === rhs.error;
}

function cloneSnapshot(): ProviderInstallProgressSnapshot {
  const snapshot: ProviderInstallProgressSnapshot = {};
  for (const [providerId, session] of installsByProviderId.entries()) {
    snapshot[providerId] = { ...session };
  }
  return snapshot;
}

function emitChange() {
  if (listeners.size === 0) return;
  const snapshot = cloneSnapshot();
  for (const listener of listeners) {
    listener(snapshot);
  }
}

export function getProviderInstallProgressSnapshot(): ProviderInstallProgressSnapshot {
  return cloneSnapshot();
}

export function subscribeProviderInstallProgress(listener: Listener): () => void {
  listeners.add(listener);
  listener(cloneSnapshot());
  return () => {
    listeners.delete(listener);
  };
}

export function upsertProviderInstallProgress(
  providerId: string,
  session: Omit<ProviderInstallProgressSession, "updatedAtMs"> & { updatedAtMs?: number },
): void {
  if (!providerId || !session.installId) return;
  const nextSession: ProviderInstallProgressSession = {
    ...session,
    updatedAtMs: session.updatedAtMs ?? Date.now(),
  };
  const existing = installsByProviderId.get(providerId);
  if (sameSession(existing, nextSession)) {
    return;
  }
  installsByProviderId.set(providerId, nextSession);
  emitChange();
}

export function removeProviderInstallProgress(providerId: string): void {
  if (!providerId) return;
  if (installsByProviderId.delete(providerId)) {
    emitChange();
  }
}

export function clearProviderInstallProgress(): void {
  if (installsByProviderId.size === 0) return;
  installsByProviderId.clear();
  emitChange();
}
