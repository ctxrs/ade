import Constants from "expo-constants";
import * as Notifications from "expo-notifications";
import { Platform } from "react-native";

import type { ConnectionConfig } from "../api/client";
import { registerMobileDevice } from "../api/client";
import { loadOrCreateDeviceIdentity } from "./deviceIdentity";

const legacyProjectId = (Constants as Record<string, any>).easConfig?.projectId;
const projectId =
  ((Constants.expoConfig?.extra as Record<string, any> | undefined)?.eas?.projectId as
    | string
    | undefined) ?? legacyProjectId;

export async function syncDeviceRegistration(conn: ConnectionConfig): Promise<void> {
  const identity = await loadOrCreateDeviceIdentity();
  await Notifications.requestPermissionsAsync();
  let pushToken: string | undefined;
  const isExpoGo = Constants.appOwnership === "expo";
  if (!isExpoGo) {
    try {
      const expoPush = await Notifications.getExpoPushTokenAsync(
        projectId ? { projectId } : undefined,
      );
      pushToken = expoPush.data;
    } catch (err) {
      console.warn("[deviceRegistration] failed to get Expo push token", err);
    }
  } else {
    console.info("[deviceRegistration] Skipping push token registration inside Expo Go");
  }
  await registerMobileDevice(conn, {
    device_id: identity.deviceId,
    device_label: Constants.deviceName ?? Platform.OS,
    platform: Platform.OS,
    push_token: pushToken,
    push_provider: pushToken ? "expo" : undefined,
    public_key: identity.publicKey,
    app_version: Constants.expoConfig?.version,
  });
}
