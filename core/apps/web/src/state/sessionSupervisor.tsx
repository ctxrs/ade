import React, { createContext, useContext, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { SessionSupervisor, type SessionCacheEntry, type SessionSupervisorSnapshot } from "./sessionSupervisorCore";

export { SessionSupervisor };
export type { SessionCacheEntry, SessionSupervisorSnapshot };

type OpenOptions = Parameters<SessionSupervisor["openSession"]>[1];

const SessionSupervisorContext = createContext<SessionSupervisor | null>(null);

export function SessionSupervisorProvider({ children }: { children: React.ReactNode }) {
  const supRef = useRef<SessionSupervisor | null>(null);
  if (!supRef.current) {
    supRef.current = new SessionSupervisor();
  }

  return (
    <SessionSupervisorContext.Provider value={supRef.current}>
      {children}
    </SessionSupervisorContext.Provider>
  );
}

export function useSessionSupervisor() {
  const sup = useContext(SessionSupervisorContext);
  if (!sup) throw new Error("SessionSupervisorProvider missing");
  return sup;
}

export function useSessionCacheSnapshot(): SessionSupervisorSnapshot {
  const sup = useSessionSupervisor();
  return useSyncExternalStore(sup.subscribe, sup.getSnapshot, sup.getSnapshot);
}

export function useSessionEntry(sessionId: string): SessionCacheEntry | null {
  const snap = useSessionCacheSnapshot();
  return snap.sessions[String(sessionId)] ?? null;
}

export function useOpenSession(sessionId: string, opts?: OpenOptions) {
  const sup = useSessionSupervisor();
  const isOptimisticSessionId = useMemo(() => String(sessionId || "").startsWith("optimistic-session-"), [sessionId]);
  const stableOpts = useMemo(
    () => ({
      watchDiff: opts?.watchDiff ?? false,
      force: opts?.force ?? false,
      silent: opts?.silent ?? false,
    }),
    [opts?.watchDiff, opts?.force, opts?.silent],
  );
  useEffect(() => {
    if (!sessionId) return;
    if (isOptimisticSessionId) return;
    return sup.openSession(String(sessionId), stableOpts);
  }, [sup, sessionId, stableOpts, isOptimisticSessionId]);
}
