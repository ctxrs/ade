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
