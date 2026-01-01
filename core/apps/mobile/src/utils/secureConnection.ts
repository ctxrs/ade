import { loadOrCreateDeviceIdentity } from "../state/deviceIdentity";
import { deriveSharedKey } from "./e2ee";

export type SecureConnectionContext = {
  deviceId: string;
  key: Uint8Array;
};

export const getSecureConnectionContext = async (
  deviceId: string,
  daemonPublicKey: string,
): Promise<SecureConnectionContext> => {
  const identity = await loadOrCreateDeviceIdentity();
  if (identity.deviceId !== deviceId) {
    throw new Error("Device identity mismatch; re-scan the QR code.");
  }
  const key = deriveSharedKey(identity.secretKey, daemonPublicKey, deviceId);
  return { deviceId, key };
};

