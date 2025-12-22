import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";

import type { ConnectionConfig } from "../api/client";
import { clearConnectionConfig, loadConnectionConfig, saveConnectionConfig } from "./connectionStorage";

type ConnectionState = {
  loading: boolean;
  config: ConnectionConfig | null;
  setConnection: (cfg: ConnectionConfig) => Promise<void>;
  clearConnection: () => Promise<void>;
};

const ConnectionContext = createContext<ConnectionState | undefined>(undefined);

export const ConnectionProvider: React.FC<React.PropsWithChildren> = ({ children }) => {
  const [loading, setLoading] = useState(true);
  const [config, setConfig] = useState<ConnectionConfig | null>(null);

  useEffect(() => {
    let mounted = true;
    (async () => {
      const stored = await loadConnectionConfig();
      if (stored && mounted) {
        setConfig(stored);
      }
      if (mounted) setLoading(false);
    })();
    return () => {
      mounted = false;
    };
  }, []);

  const setConnection = useCallback(async (cfg: ConnectionConfig) => {
    setConfig(cfg);
    await saveConnectionConfig(cfg);
  }, []);

  const clearConnection = useCallback(async () => {
    setConfig(null);
    await clearConnectionConfig();
  }, []);

  const value = useMemo<ConnectionState>(
    () => ({
      loading,
      config,
      setConnection,
      clearConnection,
    }),
    [loading, config, setConnection, clearConnection],
  );

  return <ConnectionContext.Provider value={value}>{children}</ConnectionContext.Provider>;
};

export const useConnection = (): ConnectionState => {
  const ctx = useContext(ConnectionContext);
  if (!ctx) throw new Error("useConnection must be used within ConnectionProvider");
  return ctx;
};
