import * as SecureStore from "expo-secure-store";

import type { ConnectionConfig } from "../api/client";

const STORAGE_KEY = "contextMobileConnection.v1";

export const loadConnectionConfig = async (): Promise<ConnectionConfig | null> => {
  try {
    const stored = await SecureStore.getItemAsync(STORAGE_KEY);
    if (!stored) return null;
    const parsed = JSON.parse(stored) as ConnectionConfig;
    if (!parsed?.baseUrl || !parsed?.token) return null;
    return parsed;
  } catch {
    return null;
  }
};

export const saveConnectionConfig = async (cfg: ConnectionConfig): Promise<void> => {
  await SecureStore.setItemAsync(STORAGE_KEY, JSON.stringify(cfg));
};

export const clearConnectionConfig = async (): Promise<void> => {
  await SecureStore.deleteItemAsync(STORAGE_KEY);
};
