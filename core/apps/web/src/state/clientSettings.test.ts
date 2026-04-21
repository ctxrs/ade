import { beforeEach, describe, expect, it, vi } from "vitest";

const uiStateGet = vi.hoisted(() => vi.fn());
const uiStateSet = vi.hoisted(() => vi.fn());
const uiStateDelete = vi.hoisted(() => vi.fn());

vi.mock("./uiStateStore", () => ({
  uiStateGet,
  uiStateSet,
  uiStateDelete,
}));

const loadModule = async () => {
  vi.resetModules();
  return import("./clientSettings");
};

describe("clientSettings", () => {
  beforeEach(() => {
    uiStateGet.mockReset();
    uiStateSet.mockReset();
    uiStateDelete.mockReset();
  });

  it("loads persisted v2 settings", async () => {
    uiStateGet.mockImplementation(async (key: string) => {
      if (key === "client.settings.v2") {
        return {
          v: 2,
          desktopNotifications: {
            turnCompleted: false,
            turnFailed: true,
            badgeUnreadCount: false,
          },
        };
      }
      return null;
    });
    const { loadClientSettings } = await loadModule();
    const state = await loadClientSettings();

    expect(uiStateGet).toHaveBeenCalledWith("client.settings.v2");
    expect(state.settings.desktopNotifications).toEqual({
      turnCompleted: false,
      turnFailed: true,
      badgeUnreadCount: false,
    });
  });

  it("migrates persisted v1 settings into v2", async () => {
    uiStateGet.mockImplementation(async (key: string) => {
      if (key === "client.settings.v2") return null;
      if (key === "client.settings.v1") {
        return {
          v: 1,
          desktopNotifications: { turnCompleted: false },
        };
      }
      return null;
    });
    const { loadClientSettings } = await loadModule();
    const state = await loadClientSettings();

    expect(state.settings).toEqual({
      v: 2,
      desktopNotifications: {
        turnCompleted: false,
        turnFailed: false,
        badgeUnreadCount: false,
      },
    });
    expect(uiStateSet).toHaveBeenCalledWith("client.settings.v2", state.settings);
    expect(uiStateDelete).toHaveBeenCalledWith("client.settings.v1");
  });

  it("uses enabled defaults for new installs", async () => {
    uiStateGet.mockResolvedValue(null);
    const { loadClientSettings } = await loadModule();
    const state = await loadClientSettings();

    expect(state.settings.desktopNotifications).toEqual({
      turnCompleted: true,
      turnFailed: true,
      badgeUnreadCount: true,
    });
  });

  it("persists v2 updates", async () => {
    uiStateGet.mockResolvedValue(null);
    const { updateClientSettings } = await loadModule();

    await updateClientSettings({
      desktopNotifications: {
        turnCompleted: false,
        turnFailed: true,
        badgeUnreadCount: false,
      },
    });

    expect(uiStateSet).toHaveBeenCalledWith("client.settings.v2", {
      v: 2,
      desktopNotifications: {
        turnCompleted: false,
        turnFailed: true,
        badgeUnreadCount: false,
      },
    });
  });
});
