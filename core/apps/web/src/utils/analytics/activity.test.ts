import { beforeEach, describe, expect, it, vi } from "vitest";

const { captureProductEventMock } = vi.hoisted(() => ({
  captureProductEventMock: vi.fn(),
}));

vi.mock("./client", () => ({
  captureProductEvent: captureProductEventMock,
}));

import { trackFirstTurnCompleted, trackFirstTurnSubmitted } from "./activity";

describe("analytics events", () => {
  beforeEach(() => {
    captureProductEventMock.mockReset();
    window.localStorage.clear();
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
});
