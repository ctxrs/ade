import { beforeEach, describe, expect, it, vi } from "vitest";
import { applyTurnOutcomeEffects } from "./turnOutcomeEffects";
import { resetTurnOutcomeTrackingForTests } from "../../utils/analytics/turnOutcomeDedup";

const sendDesktopNotification = vi.hoisted(() => vi.fn());
const isAppInForeground = vi.hoisted(() => vi.fn());
const getClientSettings = vi.hoisted(() => vi.fn());
const trackTurnCompleted = vi.hoisted(() => vi.fn());
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
  trackTurnCompleted,
  trackProviderRunCompleted,
  trackFirstTurnCompleted,
}));

describe("turnOutcomeEffects", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetTurnOutcomeTrackingForTests();
    isAppInForeground.mockReturnValue(false);
    getClientSettings.mockReturnValue({
      v: 1,
      desktopNotifications: { turnCompleted: true },
    });
  });

  it("tracks analytics during replay but skips notifications", () => {
    applyTurnOutcomeEffects({
      notify: false,
      sessionId: "session-1",
      turnId: "turn-1",
      providerId: "codex",
      modelId: "gpt-5",
      title: "Demo session",
      previousStatus: "running",
      nextStatus: "completed",
    });

    expect(trackTurnCompleted).toHaveBeenCalledTimes(1);
    expect(trackProviderRunCompleted).toHaveBeenCalledTimes(1);
    expect(trackFirstTurnCompleted).toHaveBeenCalledTimes(1);
    expect(sendDesktopNotification).not.toHaveBeenCalled();
  });

  it("tracks terminal outcomes and notifies only on completed turns", () => {
    applyTurnOutcomeEffects({
      notify: true,
      sessionId: "session-1",
      turnId: "turn-1",
      providerId: "codex",
      modelId: "gpt-5",
      sessionKind: "subagent",
      startedAt: "2026-03-10T00:00:00.000Z",
      completedAt: "2026-03-10T00:00:42.000Z",
      title: "Demo session",
      previousStatus: "running",
      nextStatus: "completed",
    });

    expect(trackTurnCompleted).toHaveBeenCalledWith({
      providerId: "codex",
      modelId: "gpt-5",
      reasoningEffort: undefined,
      executionEnvironment: undefined,
      status: "completed",
      durationMs: 42000,
      sessionKind: "subagent",
      metrics: undefined,
    });
    expect(trackProviderRunCompleted).toHaveBeenCalledWith({
      providerId: "codex",
      modelId: "gpt-5",
      status: "completed",
      durationMs: 42000,
      sessionKind: "subagent",
    });
    expect(trackFirstTurnCompleted).toHaveBeenCalledWith({
      sessionId: "session-1",
      providerId: "codex",
      status: "completed",
      sessionKind: "subagent",
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
      turnId: "turn-2",
      providerId: "codex",
      modelId: "gpt-5",
      title: "Demo session",
      previousStatus: "running",
      nextStatus: "failed",
    });

    expect(trackTurnCompleted).toHaveBeenCalledTimes(1);
    expect(trackProviderRunCompleted).toHaveBeenCalledTimes(1);
    expect(trackFirstTurnCompleted).toHaveBeenCalledTimes(1);
    expect(sendDesktopNotification).not.toHaveBeenCalled();
  });

  it("deduplicates analytics for the same terminal turn across replay and live updates", () => {
    applyTurnOutcomeEffects({
      notify: false,
      sessionId: "session-1",
      turnId: "turn-3",
      providerId: "codex",
      modelId: "gpt-5",
      title: "Demo session",
      previousStatus: "running",
      nextStatus: "completed",
    });
    applyTurnOutcomeEffects({
      notify: true,
      sessionId: "session-1",
      turnId: "turn-3",
      providerId: "codex",
      modelId: "gpt-5",
      title: "Demo session",
      previousStatus: "running",
      nextStatus: "completed",
    });

    expect(trackTurnCompleted).toHaveBeenCalledTimes(1);
    expect(trackProviderRunCompleted).toHaveBeenCalledTimes(1);
    expect(trackFirstTurnCompleted).toHaveBeenCalledTimes(1);
    expect(sendDesktopNotification).toHaveBeenCalledTimes(1);
  });
});
