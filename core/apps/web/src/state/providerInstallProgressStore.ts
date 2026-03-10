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

export const UNKNOWN_PROVIDER_INSTALL_TARGET = "__unknown__";

type ProviderInstallProgressTarget = Exclude<InstallInfo["target"], undefined>;

export type ProviderInstallProgressTargetKey =
  | ProviderInstallProgressTarget
  | typeof UNKNOWN_PROVIDER_INSTALL_TARGET;

export type ProviderInstallProgressSnapshot = Record<
  string,
  Partial<Record<ProviderInstallProgressTargetKey, ProviderInstallProgressSession>>
>;

type Listener = (snapshot: ProviderInstallProgressSnapshot) => void;

const installsByProviderId = new Map<string, Map<ProviderInstallProgressTargetKey, ProviderInstallProgressSession>>();
const listeners = new Set<Listener>();

const toTargetKey = (
  target: InstallInfo["target"] | undefined,
): ProviderInstallProgressTargetKey => target ?? UNKNOWN_PROVIDER_INSTALL_TARGET;

const sameTarget = (
  target: InstallInfo["target"] | undefined,
  targetKey: ProviderInstallProgressTargetKey,
): boolean => toTargetKey(target) === targetKey;

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
  for (const [providerId, sessionsByTarget] of installsByProviderId.entries()) {
    snapshot[providerId] = Object.fromEntries(
      Array.from(sessionsByTarget.entries()).map(([targetKey, session]) => [targetKey, { ...session }]),
    ) as Partial<Record<ProviderInstallProgressTargetKey, ProviderInstallProgressSession>>;
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
  const targetKey = toTargetKey(session.target);
  const nextSession: ProviderInstallProgressSession = {
    ...session,
    updatedAtMs: session.updatedAtMs ?? Date.now(),
  };
  let sessionsByTarget = installsByProviderId.get(providerId);
  if (!sessionsByTarget) {
    sessionsByTarget = new Map<ProviderInstallProgressTargetKey, ProviderInstallProgressSession>();
    installsByProviderId.set(providerId, sessionsByTarget);
  }

  let changed = false;
  if (targetKey !== UNKNOWN_PROVIDER_INSTALL_TARGET) {
    const unknownSession = sessionsByTarget.get(UNKNOWN_PROVIDER_INSTALL_TARGET);
    if (unknownSession?.installId === nextSession.installId) {
      sessionsByTarget.delete(UNKNOWN_PROVIDER_INSTALL_TARGET);
      changed = true;
    }
  }

  const existing = sessionsByTarget.get(targetKey);
  if (!changed && sameSession(existing, nextSession)) {
    return;
  }
  sessionsByTarget.set(targetKey, nextSession);
  emitChange();
}

export function resolveProviderInstallProgressSession(
  snapshot: ProviderInstallProgressSnapshot,
  providerId: string,
  target?: InstallInfo["target"],
): ProviderInstallProgressSession | undefined {
  const sessionsByTarget = snapshot[providerId];
  if (!sessionsByTarget) return undefined;

  if (target) {
    const exact = sessionsByTarget[toTargetKey(target)];
    if (exact) return exact;
    return sessionsByTarget[UNKNOWN_PROVIDER_INSTALL_TARGET];
  }

  const unknown = sessionsByTarget[UNKNOWN_PROVIDER_INSTALL_TARGET];
  if (unknown) return unknown;

  const sessions = Object.values(sessionsByTarget);
  if (sessions.length === 0) return undefined;
  if (sessions.length === 1) return sessions[0];
  return sessions.reduce((latest, current) => (
    current.updatedAtMs > latest.updatedAtMs ? current : latest
  ));
}

export function removeProviderInstallProgress(
  providerId: string,
  options?: { target?: InstallInfo["target"]; installId?: string },
): void {
  if (!providerId) return;
  const sessionsByTarget = installsByProviderId.get(providerId);
  if (!sessionsByTarget) return;

  const hasTargetFilter = Boolean(options && "target" in options);
  let changed = false;
  for (const [targetKey, session] of sessionsByTarget.entries()) {
    if (hasTargetFilter && !sameTarget(options?.target, targetKey)) {
      continue;
    }
    if (options?.installId && session.installId !== options.installId) {
      continue;
    }
    sessionsByTarget.delete(targetKey);
    changed = true;
  }

  if (sessionsByTarget.size === 0) {
    installsByProviderId.delete(providerId);
  }
  if (changed) {
    emitChange();
  }
}

export function clearProviderInstallProgress(): void {
  if (installsByProviderId.size === 0) return;
  installsByProviderId.clear();
  emitChange();
}
