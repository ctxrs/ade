import { errorMessage } from "../../utils/errorMessage";
import { emitUiDiagnostic } from "../diagnosticsChannel";
import type { SubagentInvocation } from "../../api/client";
import type { InternalEntry, SessionSupportLoadErrorKey } from "./entryState";

const SUPPORT_LOAD_ERROR_LABELS: Record<SessionSupportLoadErrorKey, string> = {
  state: "session state",
  artifacts: "artifacts",
  subagentInvocations: "subagent invocations",
};

export function formatSupportLoadError(key: SessionSupportLoadErrorKey, value: unknown): string {
  const detail = String(errorMessage(value) ?? "").trim();
  if (!detail || detail === "undefined" || detail === "null" || detail === "[object Object]") {
    return `Failed to load ${SUPPORT_LOAD_ERROR_LABELS[key]}.`;
  }
  if (detail.startsWith("Failed to load ")) {
    return detail;
  }
  return `Failed to load ${SUPPORT_LOAD_ERROR_LABELS[key]}: ${detail}`;
}

type SessionStateLoadStatus = {
  stateLoaded: boolean;
  stateLoading: boolean;
  stateRev?: number;
  stateAppliedRev?: number;
};

type SubagentInvocationsLoadStatus = {
  subagentInvocationsLoaded?: boolean;
  subagentInvocationsLoading: boolean;
  subagentInvocationsAppliedRev?: number;
};

export function shouldFetchSessionState(
  entry: SessionStateLoadStatus,
  opts?: { force?: boolean },
): boolean {
  if (entry.stateLoading) return false;
  if (opts?.force) return true;
  if (!entry.stateLoaded) return true;
  if (
    typeof entry.stateRev === "number" &&
    typeof entry.stateAppliedRev === "number" &&
    entry.stateAppliedRev < entry.stateRev
  ) {
    return true;
  }
  if (typeof entry.stateRev === "number" && typeof entry.stateAppliedRev !== "number") {
    return true;
  }
  return false;
}

export function shouldFetchSubagentInvocations(
  entry: SubagentInvocationsLoadStatus,
  requestedStateRev: number | undefined,
  opts?: { force?: boolean },
): boolean {
  if (entry.subagentInvocationsLoading) return false;
  if (opts?.force) return true;
  if (!entry.subagentInvocationsLoaded) return true;
  if (
    typeof requestedStateRev === "number" &&
    typeof entry.subagentInvocationsAppliedRev === "number" &&
    entry.subagentInvocationsAppliedRev < requestedStateRev
  ) {
    return true;
  }
  if (
    typeof requestedStateRev === "number" &&
    typeof entry.subagentInvocationsAppliedRev !== "number"
  ) {
    return true;
  }
  return false;
}

export function deriveSupportFreshnessKey(
  requestedStateRev: number | undefined,
  freshnessEpoch: number,
): string {
  if (typeof requestedStateRev === "number") {
    return `rev:${requestedStateRev}`;
  }
  return `epoch:${freshnessEpoch}`;
}

export function adoptLoadedStateRevision(
  stateLoaded: boolean,
  currentAppliedRev: number | undefined,
  nextKnownRev: number | undefined,
): number | undefined {
  if (!stateLoaded || typeof nextKnownRev !== "number") return currentAppliedRev;
  if (typeof currentAppliedRev === "number") return currentAppliedRev;
  return nextKnownRev;
}

type SupportLoadSyncDeps = {
  resolveRequestedStateRev(entry: InternalEntry): number | undefined;
  ensureState(entry: InternalEntry): Promise<void>;
  ensureSubagentInvocations(entry: InternalEntry): Promise<void>;
};

export function syncSupportLoadsForOpenSession(
  entry: InternalEntry,
  deps: SupportLoadSyncDeps,
): void {
  if (entry.refCount <= 0) return;
  const requestedStateRev = deps.resolveRequestedStateRev(entry);
  const freshnessKey = deriveSupportFreshnessKey(requestedStateRev, entry.supportFreshnessEpoch);
  if (entry.stateAutoLoadKey !== freshnessKey && shouldFetchSessionState(entry)) {
    entry.stateAutoLoadKey = freshnessKey;
    void deps.ensureState(entry);
  }
  if (
    entry.subagentAutoLoadKey !== freshnessKey &&
    shouldFetchSubagentInvocations(entry, requestedStateRev)
  ) {
    entry.subagentAutoLoadKey = freshnessKey;
    void deps.ensureSubagentInvocations(entry);
  }
}

type SupportLoadInvalidationDeps = {
  resolveRequestedStateRev(entry: InternalEntry): number | undefined;
  subagentInvocationsCacheBySessionId: Map<
    string,
    { invocations: SubagentInvocation[]; stateRev: number }
  >;
};

export function invalidateSupportLoadsWithoutAuthoritativeRevision(
  entry: InternalEntry,
  deps: SupportLoadInvalidationDeps,
): void {
  if (typeof deps.resolveRequestedStateRev(entry) === "number") return;
  entry.supportFreshnessEpoch += 1;
  if (!entry.stateLoading) {
    entry.stateLoaded = false;
    entry.stateAppliedRev = undefined;
  }
  if (!entry.subagentInvocationsLoading) {
    entry.subagentInvocationsLoaded = false;
    entry.subagentInvocationsAppliedRev = undefined;
  }
  deps.subagentInvocationsCacheBySessionId.delete(entry.sessionId);
}

export function adoptLoadedSubagentInvocationsRevision(
  entry: InternalEntry,
  stateRev: number,
  subagentInvocationsCacheBySessionId: Map<
    string,
    { invocations: SubagentInvocation[]; stateRev: number }
  >,
): void {
  if (!entry.subagentInvocationsLoaded) return;
  if (typeof entry.subagentInvocationsAppliedRev === "number") return;
  entry.subagentInvocationsAppliedRev = stateRev;
  subagentInvocationsCacheBySessionId.set(entry.sessionId, {
    invocations: entry.subagentInvocations.slice(),
    stateRev,
  });
}

export function clearSupportLoadError(
  entry: InternalEntry,
  key: SessionSupportLoadErrorKey,
): void {
  if (!entry.loadErrors[key]) return;
  delete entry.loadErrors[key];
}

export function setSupportLoadError(
  entry: InternalEntry,
  key: SessionSupportLoadErrorKey,
  value: unknown,
): void {
  const message = formatSupportLoadError(key, value);
  emitUiDiagnostic({
    source: "session_supervisor",
    code: `session.${key}_load_failed`,
    severity: "error",
    fatal: false,
    message,
    context: {
      sessionId: entry.sessionId,
      mode: entry.mode ?? null,
      target: key,
    },
  });
  entry.loadErrors[key] = message;
}
