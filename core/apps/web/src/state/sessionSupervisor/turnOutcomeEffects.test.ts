import { beforeEach, describe, expect, it, vi } from "vitest";
import { applyTurnOutcomeEffects } from "./turnOutcomeEffects";
import { resetTurnOutcomeTrackingForTests } from "../../utils/analytics/turnOutcomeDedup";

const sendDesktopNotification = vi.hoisted(() => vi.fn());
const isAppInForeground = vi.hoisted(() => vi.fn());
const getClientSettingsState = vi.hoisted(() => vi.fn());
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
  getClientSettingsState,
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
    getClientSettingsState.mockReturnValue({
      loaded: true,
      settings: {
        v: 2,
        desktopNotifications: {
          turnCompleted: true,
          turnFailed: true,
          badgeUnreadCount: true,
        },
      },
    });
  });

  it("tracks analytics during replay but skips notifications", () => {
    applyTurnOutcomeEffects({
      notify: false,
      sessionId: "session-1",
      taskId: "task-1",
      workspaceId: "workspace-1",
      turnId: "turn-1",
      providerId: "codex",
      modelId: "gpt-5",
      previousStatus: "running",
      nextStatus: "completed",
    });

    expect(trackTurnCompleted).toHaveBeenCalledTimes(1);
    expect(trackProviderRunCompleted).toHaveBeenCalledTimes(1);
    expect(trackFirstTurnCompleted).toHaveBeenCalledTimes(1);
    expect(sendDesktopNotification).not.toHaveBeenCalled();
  });

  it("tracks terminal outcomes and notifies only for completed primary turns", () => {
    applyTurnOutcomeEffects({
      notify: true,
      sessionId: "session-1",
      taskId: "task-1",
      workspaceId: "workspace-1",
      turnId: "turn-1",
      providerId: "codex",
      modelId: "gpt-5",
      sessionKind: "primary",
      startedAt: "2026-03-10T00:00:00.000Z",
      completedAt: "2026-03-10T00:00:42.000Z",
      notificationTitle: "Fix login race",
      notificationBody: "Added retry logic around token refresh.",
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
      sessionKind: "primary",
      metrics: undefined,
    });
    expect(trackProviderRunCompleted).toHaveBeenCalledWith({
      providerId: "codex",
      modelId: "gpt-5",
      status: "completed",
      durationMs: 42000,
      sessionKind: "primary",
    });
    expect(trackFirstTurnCompleted).toHaveBeenCalledWith({
      sessionId: "session-1",
      providerId: "codex",
      status: "completed",
      sessionKind: "primary",
    });
    expect(sendDesktopNotification).toHaveBeenCalledWith({
      kind: "turn_completed",
      title: "Fix login race",
      body: "Added retry logic around token refresh.",
      workspaceId: "workspace-1",
      taskId: "task-1",
      sessionId: "session-1",
    });
  });

  it("sends error notifications for failed primary turns", () => {
    applyTurnOutcomeEffects({
      notify: true,
      sessionId: "session-1",
      taskId: "task-1",
      workspaceId: "workspace-1",
      turnId: "turn-2",
      providerId: "codex",
      modelId: "gpt-5",
      sessionKind: "primary",
      notificationTitle: "Fix login race",
      notificationBody: "The browser cookie still disappears after refresh.",
      previousStatus: "running",
      nextStatus: "failed",
    });

    expect(trackTurnCompleted).toHaveBeenCalledTimes(1);
    expect(trackProviderRunCompleted).toHaveBeenCalledTimes(1);
    expect(trackFirstTurnCompleted).toHaveBeenCalledTimes(1);
    expect(sendDesktopNotification).toHaveBeenCalledWith({
      kind: "turn_failed",
      title: "Fix login race",
      body: "The browser cookie still disappears after refresh.",
      workspaceId: "workspace-1",
      taskId: "task-1",
      sessionId: "session-1",
    });
  });

  it("does not notify until client settings finish loading", () => {
    getClientSettingsState.mockReturnValue({
      loaded: false,
      settings: {
        v: 2,
        desktopNotifications: {
          turnCompleted: true,
          turnFailed: true,
          badgeUnreadCount: true,
        },
      },
    });

    applyTurnOutcomeEffects({
      notify: true,
      sessionId: "session-1",
      taskId: "task-1",
      workspaceId: "workspace-1",
      turnId: "turn-4",
      sessionKind: "primary",
      notificationTitle: "Fix login race",
      notificationBody: "Added retry logic around token refresh.",
      previousStatus: "running",
      nextStatus: "completed",
    });

    expect(sendDesktopNotification).not.toHaveBeenCalled();
  });

  it("never falls back to the session title for the notification title", () => {
    applyTurnOutcomeEffects({
      notify: true,
      sessionId: "session-1",
      taskId: "task-1",
      workspaceId: "workspace-1",
      turnId: "turn-5",
      sessionKind: "primary",
      title: "Demo session",
      previousStatus: "running",
      nextStatus: "completed",
    });

    expect(sendDesktopNotification).toHaveBeenCalledWith({
      kind: "turn_completed",
      title: "Turn completed",
      body: undefined,
      workspaceId: "workspace-1",
      taskId: "task-1",
      sessionId: "session-1",
    });
  });

  it("does not notify for subagent turns", () => {
    applyTurnOutcomeEffects({
      notify: true,
      sessionId: "session-1",
      taskId: "task-1",
      workspaceId: "workspace-1",
      turnId: "turn-1",
      sessionKind: "subagent",
      previousStatus: "running",
      nextStatus: "completed",
    });

    expect(sendDesktopNotification).not.toHaveBeenCalled();
  });

  it("deduplicates analytics for the same terminal turn across replay and live updates", () => {
    applyTurnOutcomeEffects({
      notify: false,
      sessionId: "session-1",
      taskId: "task-1",
      workspaceId: "workspace-1",
      turnId: "turn-3",
      providerId: "codex",
      modelId: "gpt-5",
      previousStatus: "running",
      nextStatus: "completed",
    });
    applyTurnOutcomeEffects({
      notify: true,
      sessionId: "session-1",
      taskId: "task-1",
      workspaceId: "workspace-1",
      turnId: "turn-3",
      providerId: "codex",
      modelId: "gpt-5",
      sessionKind: "primary",
      previousStatus: "running",
      nextStatus: "completed",
    });

    expect(trackTurnCompleted).toHaveBeenCalledTimes(1);
    expect(trackProviderRunCompleted).toHaveBeenCalledTimes(1);
    expect(trackFirstTurnCompleted).toHaveBeenCalledTimes(1);
    expect(sendDesktopNotification).toHaveBeenCalledTimes(1);
  });
});
