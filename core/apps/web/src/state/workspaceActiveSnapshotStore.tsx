import React, { createContext, useContext, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import type { WorkspaceActiveSnapshotEvent } from "@ctx/types";
import {
  WorkspaceActiveSnapshotStoreImpl,
  type WorkspaceActiveSnapshotEventSource,
  type WorkspaceActiveSnapshotState,
  type WorkspaceActiveSnapshotItem,
} from "./workspaceActiveSnapshotStoreCore";

export type { WorkspaceActiveSnapshotEventSource, WorkspaceActiveSnapshotItem, WorkspaceActiveSnapshotState };

const WorkspaceActiveSnapshotContext = createContext<WorkspaceActiveSnapshotStoreImpl | null>(null);

const shouldExposeE2E = (): boolean => {
  if (typeof window === "undefined") return false;
  const params = new URLSearchParams(window.location.search);
  return params.get("ctxE2E") === "1";
};

export function WorkspaceActiveSnapshotProvider({
  workspaceId,
  children,
}: {
  workspaceId: string;
  children: React.ReactNode;
}) {
  const storeRef = useRef<WorkspaceActiveSnapshotStoreImpl | null>(null);
  const lastWorkspaceRef = useRef<string | null>(null);
  if (!storeRef.current || lastWorkspaceRef.current !== workspaceId) {
    storeRef.current?.destroy();
    storeRef.current = new WorkspaceActiveSnapshotStoreImpl(workspaceId);
    lastWorkspaceRef.current = workspaceId;
  }

  useEffect(() => {
    storeRef.current?.init();
    return () => storeRef.current?.destroy();
  }, [workspaceId]);

  useEffect(() => {
    if (!shouldExposeE2E()) return;
    const store = storeRef.current;
    if (!store) return;
    const win = window as any;
    win.__ctxE2E ??= {};
    win.__ctxE2E.getSessionHeadMessages = (sessionId: string) => {
      const head = storeRef.current?.getSessionHeadSnapshot(sessionId);
      return head?.messages?.map((message) => message.content) ?? [];
    };
    win.__ctxE2E.getSessionLastEventSeq = (sessionId: string) => {
      const head = storeRef.current?.getSessionHeadSnapshot(sessionId);
      return head?.last_event_seq ?? null;
    };
    return () => {
      if (!win.__ctxE2E) return;
      delete win.__ctxE2E.getSessionHeadMessages;
      delete win.__ctxE2E.getSessionLastEventSeq;
    };
  }, [workspaceId]);

  return (
    <WorkspaceActiveSnapshotContext.Provider value={storeRef.current}>
      {children}
    </WorkspaceActiveSnapshotContext.Provider>
  );
}

export function useWorkspaceActiveSnapshotStore() {
  const store = useContext(WorkspaceActiveSnapshotContext);
  if (!store) throw new Error("WorkspaceActiveSnapshotProvider missing");
  return store;
}

export function useWorkspaceActiveSnapshotSnapshot(): WorkspaceActiveSnapshotState {
  const store = useWorkspaceActiveSnapshotStore();
  return useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
}

export function useWorkspaceActiveSnapshotEvents(handler: (event: WorkspaceActiveSnapshotEvent) => void) {
  const store = useWorkspaceActiveSnapshotStore();
  const stableHandler = useMemo(() => handler, [handler]);
  useEffect(() => store.subscribeEvents(stableHandler), [store, stableHandler]);
}
