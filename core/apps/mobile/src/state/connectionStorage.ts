import AsyncStorage from "@react-native-async-storage/async-storage";

import type { ConnectionConfig } from "../api/client";

const STORAGE_KEY = "contextMobileConnection.v1";

type SecureStoreModule = typeof import("expo-secure-store");

let secureStorePromise: Promise<SecureStoreModule | null> | null = null;

const loadSecureStore = async (): Promise<SecureStoreModule | null> => {
  if (!secureStorePromise) {
    secureStorePromise = import("expo-secure-store")
      .then((mod) => mod)
      .catch(() => null);
  }
  return secureStorePromise;
};

const isSecureStoreAvailable = async (): Promise<boolean> => {
  const secureStore = await loadSecureStore();
  if (!secureStore?.isAvailableAsync) return false;
  try {
    return await secureStore.isAvailableAsync();
  } catch {
    return false;
  }
};

const readValue = async (key: string): Promise<string | null> => {
  if (await isSecureStoreAvailable()) {
    const secureStore = await loadSecureStore();
    if (secureStore?.getItemAsync) {
      try {
        return await secureStore.getItemAsync(key);
      } catch {
        // fall through to AsyncStorage
      }
    }
  }
  return AsyncStorage.getItem(key);
};

const writeValue = async (key: string, value: string): Promise<void> => {
  if (await isSecureStoreAvailable()) {
    const secureStore = await loadSecureStore();
    if (secureStore?.setItemAsync) {
      try {
        await secureStore.setItemAsync(key, value);
        return;
      } catch {
        // fall through to AsyncStorage
      }
    }
  }
  await AsyncStorage.setItem(key, value);
};

const removeValue = async (key: string): Promise<void> => {
  if (await isSecureStoreAvailable()) {
    const secureStore = await loadSecureStore();
    if (secureStore?.deleteItemAsync) {
      try {
        await secureStore.deleteItemAsync(key);
        return;
      } catch {
        // fall through to AsyncStorage
      }
    }
  }
  await AsyncStorage.removeItem(key);
};

export const loadConnectionConfig = async (): Promise<ConnectionConfig | null> => {
  try {
    const stored = await readValue(STORAGE_KEY);
    if (!stored) return null;
    const parsed = JSON.parse(stored) as ConnectionConfig;
    if (!parsed?.baseUrl) return null;
    if (parsed.daemonPublicKey) {
      if (!parsed.deviceId) return null;
      return parsed;
    }
    if (!parsed.token) return null;
    return parsed;
  } catch {
    return null;
  }
};

export const saveConnectionConfig = async (cfg: ConnectionConfig): Promise<void> => {
  await writeValue(STORAGE_KEY, JSON.stringify(cfg));
};

export const clearConnectionConfig = async (): Promise<void> => {
  await removeValue(STORAGE_KEY);
};
