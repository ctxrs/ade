import React, { createContext, useContext, useEffect, useMemo, useState } from "react";

import { useConnection } from "./ConnectionProvider";

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
    }
  }, [config]);

  const value = useMemo<WorkspaceSelectionState>(
    () => ({
      workspaceId,
      workspaceName,
      setWorkspace: (id: string, name: string) => {
        setWorkspaceId(id);
        setWorkspaceName(name);
      },
      clearWorkspace: () => {
        setWorkspaceId(null);
        setWorkspaceName(null);
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
