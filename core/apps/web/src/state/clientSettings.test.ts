import { beforeEach, describe, expect, it, vi } from "vitest";

const uiStateGet = vi.hoisted(() => vi.fn());
const uiStateSet = vi.hoisted(() => vi.fn());

vi.mock("./uiStateStore", () => ({
  uiStateGet,
  uiStateSet,
}));

const loadModule = async () => {
  vi.resetModules();
  return import("./clientSettings");
};

describe("clientSettings", () => {
  beforeEach(() => {
    uiStateGet.mockReset();
    uiStateSet.mockReset();
  });

  it("loads persisted settings", async () => {
    uiStateGet.mockResolvedValue({
      v: 1,
      desktopNotifications: { turnCompleted: true },
    });
    const { loadClientSettings } = await loadModule();
    const state = await loadClientSettings();

    expect(uiStateGet).toHaveBeenCalledWith("client.settings.v1");
    expect(state.settings.desktopNotifications.turnCompleted).toBe(true);
  });

  it("persists updates", async () => {
    uiStateGet.mockResolvedValue(null);
    const { updateClientSettings } = await loadModule();

    await updateClientSettings({ desktopNotifications: { turnCompleted: true } });
    expect(uiStateSet).toHaveBeenCalledWith("client.settings.v1", {
      v: 1,
      desktopNotifications: { turnCompleted: true },
    });
  });
});
