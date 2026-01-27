import { isDesktopApp } from "./desktop";

export type DesktopNotificationPayload = {
  title: string;
  body?: string;
};

export async function ensureDesktopNotificationPermission(): Promise<boolean> {
  if (!isDesktopApp()) return false;
  try {
    const mod = await import("@tauri-apps/plugin-notification");
    const granted = await mod.isPermissionGranted();
    if (granted) return true;
    const res = await mod.requestPermission();
    return res === "granted";
  } catch (err) {
    console.warn("desktop notifications permission failed", err);
    return false;
  }
}

export async function sendDesktopNotification(payload: DesktopNotificationPayload): Promise<void> {
  if (!isDesktopApp()) return;
  try {
    const mod = await import("@tauri-apps/plugin-notification");
    const granted = await mod.isPermissionGranted();
    if (!granted) return;
    mod.sendNotification({ title: payload.title, body: payload.body });
  } catch (err) {
    console.warn("desktop notification send failed", err);
  }
}
