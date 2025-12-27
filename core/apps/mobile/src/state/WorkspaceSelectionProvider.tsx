import AsyncStorage from "@react-native-async-storage/async-storage";
import React, { createContext, useContext, useEffect, useMemo, useState } from "react";

import { useConnection } from "./ConnectionProvider";

const STORAGE_KEY = "contextMobileSelectedWorkspace.v1";

type WorkspaceSelectionState = {
  workspaceId: string | null;
  workspaceName: string | null;
  setWorkspace: (id: string, name: string) => void;
  clearWorkspace: () => void;
};

const WorkspaceSelectionContext = createContext<WorkspaceSelectionState | undefined>(undefined);

export const WorkspaceSelectionProvider: React.FC<React.PropsWithChildren> = ({ children }) => {
  const { config } = useConnection();
  const [workspaceId, setWorkspaceId] = useState<string | null>(null);
  const [workspaceName, setWorkspaceName] = useState<string | null>(null);

  useEffect(() => {
    if (!config) {
      setWorkspaceId(null);
      setWorkspaceName(null);
      AsyncStorage.removeItem(STORAGE_KEY).catch(() => {});
      return;
    }
    let cancelled = false;
    AsyncStorage.getItem(STORAGE_KEY)
      .then((raw) => {
        if (cancelled || !raw) return;
        const parsed = JSON.parse(raw) as { id?: string; name?: string };
        if (parsed?.id && parsed?.name) {
          setWorkspaceId(String(parsed.id));
          setWorkspaceName(String(parsed.name));
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [config]);

  const value = useMemo<WorkspaceSelectionState>(
    () => ({
      workspaceId,
      workspaceName,
      setWorkspace: (id: string, name: string) => {
        setWorkspaceId(id);
        setWorkspaceName(name);
        AsyncStorage.setItem(STORAGE_KEY, JSON.stringify({ id, name })).catch(() => {});
      },
      clearWorkspace: () => {
        setWorkspaceId(null);
        setWorkspaceName(null);
        AsyncStorage.removeItem(STORAGE_KEY).catch(() => {});
      },
    }),
    [workspaceId, workspaceName],
  );

  return (
    <WorkspaceSelectionContext.Provider value={value}>
      {children}
    </WorkspaceSelectionContext.Provider>
  );
};

export const useWorkspaceSelection = (): WorkspaceSelectionState => {
  const ctx = useContext(WorkspaceSelectionContext);
  if (!ctx) throw new Error("useWorkspaceSelection must be used within WorkspaceSelectionProvider");
  return ctx;
};
