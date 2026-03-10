import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { captureProductEventMock } = vi.hoisted(() => ({
  captureProductEventMock: vi.fn(),
}));

vi.mock("./client", () => ({
  captureProductEvent: captureProductEventMock,
}));

import {
  trackFirstTurnCompleted,
  trackFirstTurnSubmitted,
  trackWizardAbandoned,
  trackWizardCompleted,
  trackWizardStarted,
  trackWizardStepCompleted,
  trackWizardStepViewed,
  trackWorkbenchPanelToggled,
  trackWorkspaceCreateFailed,
  trackWorkspaceCreateSubmitted,
  trackWorkspaceCreateSucceeded,
  trackWorkspaceLaunchCompleted,
  trackWorkspaceRouteOpenedFromPending,
} from "./activity";

describe("analytics events", () => {
  beforeEach(() => {
    captureProductEventMock.mockReset();
    window.localStorage.clear();
    window.sessionStorage.clear();
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-10T00:00:10.000Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("deduplicates first_turn_submitted per install scope", () => {
    trackFirstTurnSubmitted({ sessionId: "s1", providerId: "codex" });
    trackFirstTurnSubmitted({ sessionId: "s2", providerId: "codex" });
    expect(captureProductEventMock).toHaveBeenCalledTimes(1);
    expect(captureProductEventMock).toHaveBeenCalledWith(
      "first_turn_submitted",
      1,
      expect.objectContaining({ provider_id: "codex" }),
    );
  });

  it("records first_turn_completed once per install scope and only for completed status", () => {
    trackFirstTurnCompleted({ sessionId: "s2", providerId: "claude", status: "failed" });
    trackFirstTurnCompleted({ sessionId: "s2", providerId: "claude", status: "completed" });
    trackFirstTurnCompleted({ sessionId: "s3", providerId: "claude", status: "completed" });
    expect(captureProductEventMock).toHaveBeenCalledTimes(1);
    expect(captureProductEventMock).toHaveBeenCalledWith(
      "first_turn_completed",
      1,
      expect.objectContaining({ provider_id: "claude", status: "completed" }),
    );
  });

  it("tracks wizard, workspace create lifecycle, and workbench panel toggle events", () => {
    trackWizardStarted({ wizardKey: "workspace_setup" });
    trackWizardStepViewed({ wizardKey: "workspace_setup", stepKey: "location", stepIndex: 0 });
    trackWizardStepCompleted({ wizardKey: "workspace_setup", stepKey: "location", stepIndex: 0 });
    trackWizardCompleted({ wizardKey: "workspace_setup", workspaceKind: "local" });
    trackWizardAbandoned({ wizardKey: "workspace_setup", lastStepKey: "source", lastStepIndex: 3 });
    trackWorkspaceCreateSubmitted({ workspaceKind: "local", source: "wizard" });
    trackWorkspaceCreateSucceeded({ workspaceKind: "local", source: "wizard" });
    trackWorkspaceCreateFailed({ workspaceKind: "remote", source: "api", failureKind: "request_error" });
    trackWorkbenchPanelToggled({ panelKey: "terminal", open: true, source: "header_button" });

    expect(captureProductEventMock).toHaveBeenCalledWith(
      "wizard_started",
      1,
      expect.objectContaining({ wizard_key: "workspace_setup" }),
    );
    expect(captureProductEventMock).toHaveBeenCalledWith(
      "workspace_create_failed",
      1,
      expect.objectContaining({
        workspace_kind: "remote",
        source: "api",
        failure_kind: "request_error",
      }),
    );
    expect(captureProductEventMock).toHaveBeenCalledWith(
      "workbench_panel_toggled",
      1,
      expect.objectContaining({
        panel_key: "terminal",
        open: true,
        source: "header_button",
      }),
    );
  });

  it("tracks click-to-ready and click-to-route metrics for workspace launch", () => {
    trackWorkspaceLaunchCompleted({
      workspaceId: "ws_123",
      workspaceKind: "local",
      executionMode: "container",
      source: "wizard",
      startedAtMs: Date.parse("2026-03-10T00:00:00.000Z"),
      result: "ready",
    });
    trackWorkspaceRouteOpenedFromPending("ws_123");

    expect(captureProductEventMock).toHaveBeenCalledWith(
      "workspace_launch_completed",
      1,
      expect.objectContaining({
        click_to_launch_ready_ms: 10000,
        execution_mode: "container",
        result: "ready",
      }),
    );
    expect(captureProductEventMock).toHaveBeenCalledWith(
      "workspace_route_opened",
      1,
      expect.objectContaining({
        click_to_workspace_route_ms: 10000,
        execution_mode: "container",
      }),
    );
    expect(captureProductEventMock).not.toHaveBeenCalledWith(
      "workspace_launch_completed",
      1,
      expect.objectContaining({ workspace_id: "ws_123" }),
    );
    expect(captureProductEventMock).not.toHaveBeenCalledWith(
      "workspace_route_opened",
      1,
      expect.objectContaining({ workspace_id: "ws_123" }),
    );
  });
});
