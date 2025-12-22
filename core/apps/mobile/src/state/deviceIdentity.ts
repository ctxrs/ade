import "react-native-get-random-values";
import * as SecureStore from "expo-secure-store";
import nacl from "tweetnacl";
import { Buffer } from "buffer";
import { v4 as uuidv4 } from "uuid";

const globalWithBuffer = globalThis as typeof globalThis & { Buffer?: typeof Buffer };
if (!globalWithBuffer.Buffer) {
  globalWithBuffer.Buffer = Buffer;
}

const DEVICE_IDENTITY_KEY = "context.mobile.device_identity.v1";

export type DeviceIdentity = {
  deviceId: string;
  publicKey: string;
  secretKey: string;
};

const bufferFrom = (bytes: Uint8Array): string => Buffer.from(bytes).toString("base64");

export const loadOrCreateDeviceIdentity = async (): Promise<DeviceIdentity> => {
  const stored = await SecureStore.getItemAsync(DEVICE_IDENTITY_KEY);
  if (stored) {
    try {
      const parsed = JSON.parse(stored) as DeviceIdentity;
      if (parsed?.deviceId && parsed?.publicKey && parsed?.secretKey) {
        return parsed;
      }
    } catch {
      // ignore and regenerate
    }
  }
  const pair = nacl.box.keyPair();
  const identity: DeviceIdentity = {
    deviceId: uuidv4(),
    publicKey: bufferFrom(pair.publicKey),
    secretKey: bufferFrom(pair.secretKey),
  };
  await SecureStore.setItemAsync(DEVICE_IDENTITY_KEY, JSON.stringify(identity));
  return identity;
};

export const clearDeviceIdentity = async (): Promise<void> => {
  await SecureStore.deleteItemAsync(DEVICE_IDENTITY_KEY);
};
