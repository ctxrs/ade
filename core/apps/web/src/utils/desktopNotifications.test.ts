import { beforeEach, describe, expect, it, vi } from "vitest";
import { ensureDesktopNotificationPermission, sendDesktopNotification } from "./desktopNotifications";

const isDesktopApp = vi.hoisted(() => vi.fn());
const isPermissionGranted = vi.hoisted(() => vi.fn());
const requestPermission = vi.hoisted(() => vi.fn());
const sendNotification = vi.hoisted(() => vi.fn());

vi.mock("./desktop", () => ({
  isDesktopApp,
}));

vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted,
  requestPermission,
  sendNotification,
}));

describe("desktopNotifications", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    isDesktopApp.mockReturnValue(true);
    isPermissionGranted.mockResolvedValue(true);
    requestPermission.mockResolvedValue("denied");
  });

  it("returns false when not running in the desktop app", async () => {
    isDesktopApp.mockReturnValue(false);
    await expect(ensureDesktopNotificationPermission()).resolves.toBe(false);
    expect(isPermissionGranted).not.toHaveBeenCalled();
  });

  it("requests permission when needed", async () => {
    isPermissionGranted.mockResolvedValue(false);
    requestPermission.mockResolvedValue("granted");

    await expect(ensureDesktopNotificationPermission()).resolves.toBe(true);
    expect(requestPermission).toHaveBeenCalledTimes(1);
  });

  it("does not send notifications when permission is denied", async () => {
    isPermissionGranted.mockResolvedValue(false);

    await sendDesktopNotification({ title: "Turn completed" });
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("sends notifications when permission is granted", async () => {
    isPermissionGranted.mockResolvedValue(true);

    await sendDesktopNotification({ title: "Turn completed", body: "Session title" });
    expect(sendNotification).toHaveBeenCalledWith({ title: "Turn completed", body: "Session title" });
  });
});
