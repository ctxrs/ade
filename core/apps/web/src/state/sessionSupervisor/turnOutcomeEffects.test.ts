import { beforeEach, describe, expect, it, vi } from "vitest";
import { applyTurnOutcomeEffects } from "./turnOutcomeEffects";

const sendDesktopNotification = vi.hoisted(() => vi.fn());
const isAppInForeground = vi.hoisted(() => vi.fn());
const getClientSettings = vi.hoisted(() => vi.fn());
const trackProviderRunCompleted = vi.hoisted(() => vi.fn());
const trackFirstTurnCompleted = vi.hoisted(() => vi.fn());

vi.mock("../../utils/desktopNotifications", () => ({
  sendDesktopNotification,
}));

vi.mock("../../utils/windowFocus", () => ({
  isAppInForeground,
}));

vi.mock("../clientSettings", () => ({
  getClientSettings,
}));

vi.mock("../../utils/analytics", () => ({
  trackProviderRunCompleted,
  trackFirstTurnCompleted,
}));

describe("turnOutcomeEffects", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    isAppInForeground.mockReturnValue(false);
    getClientSettings.mockReturnValue({
      v: 1,
      desktopNotifications: { turnCompleted: true },
    });
  });

  it("skips analytics and notifications during replay", () => {
    applyTurnOutcomeEffects({
      notify: false,
      sessionId: "session-1",
      providerId: "codex",
      modelId: "gpt-5",
      title: "Demo session",
      previousStatus: "running",
      nextStatus: "completed",
    });

    expect(trackProviderRunCompleted).not.toHaveBeenCalled();
    expect(trackFirstTurnCompleted).not.toHaveBeenCalled();
    expect(sendDesktopNotification).not.toHaveBeenCalled();
  });

  it("tracks terminal outcomes and notifies only on completed turns", () => {
    applyTurnOutcomeEffects({
      notify: true,
      sessionId: "session-1",
      providerId: "codex",
      modelId: "gpt-5",
      title: "Demo session",
      previousStatus: "running",
      nextStatus: "completed",
    });

    expect(trackProviderRunCompleted).toHaveBeenCalledWith({
      providerId: "codex",
      modelId: "gpt-5",
      status: "completed",
    });
    expect(trackFirstTurnCompleted).toHaveBeenCalledWith({
      sessionId: "session-1",
      providerId: "codex",
      status: "completed",
    });
    expect(sendDesktopNotification).toHaveBeenCalledWith({
      title: "Turn completed",
      body: "Demo session",
    });
  });

  it("tracks failed turns without sending a completion notification", () => {
    applyTurnOutcomeEffects({
      notify: true,
      sessionId: "session-1",
      providerId: "codex",
      modelId: "gpt-5",
      title: "Demo session",
      previousStatus: "running",
      nextStatus: "failed",
    });

    expect(trackProviderRunCompleted).toHaveBeenCalledTimes(1);
    expect(trackFirstTurnCompleted).toHaveBeenCalledTimes(1);
    expect(sendDesktopNotification).not.toHaveBeenCalled();
  });
});
