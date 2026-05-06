import type { SemanticTelemetryEvent } from "@ctx/types";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  recordSemanticTelemetryEvent,
  resetClientBaseTelemetryForTests,
  setSemanticTelemetryRemoteEnabled,
} from "./clientBaseTelemetry";

vi.mock("../utils/desktop", () => ({
  isDesktopApp: () => false,
}));

const fetchMock = vi.fn<(input: RequestInfo | URL, init?: RequestInit) => Promise<Response>>();

const semanticEvent = (
  eventId: string,
  overrides: Partial<SemanticTelemetryEvent> = {},
): SemanticTelemetryEvent => ({
  event_id: eventId,
  event_name: "app_opened",
  event_version: 1,
  occurred_at: "2026-05-06T12:00:00.000Z",
  plane: "product",
  delivery: "remote",
  origin_runtime: "desktop",
  origin_install_id: "install-1",
  app_version: "1.2.3",
  os: "macos",
  arch: "arm64",
  surface: "desktop",
  env_target: "remote",
  source: "test",
  properties: { launch_surface: "desktop" },
  ...overrides,
});

const response = (status: number): Response =>
  new Response(null, { status });

const postedBatchAt = (index: number): { events: SemanticTelemetryEvent[] } => {
  const init = fetchMock.mock.calls[index]?.[1];
  const rawBody = String(init?.body ?? "{}");
  return JSON.parse(rawBody) as { events: SemanticTelemetryEvent[] };
};

describe("clientBase semantic telemetry", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    fetchMock.mockReset();
    vi.stubGlobal("fetch", fetchMock);
    resetClientBaseTelemetryForTests();
  });

  afterEach(() => {
    resetClientBaseTelemetryForTests();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("retains product events and retries after a transport failure", async () => {
    fetchMock
      .mockRejectedValueOnce(new Error("network unavailable"))
      .mockResolvedValueOnce(response(204));

    recordSemanticTelemetryEvent(semanticEvent("event-1"));

    await vi.advanceTimersByTimeAsync(1_000);
    expect(fetchMock).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(1_000);
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(postedBatchAt(1).events.map((event) => event.event_id)).toEqual(["event-1"]);

    await vi.advanceTimersByTimeAsync(2_000);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("retains product events and retries after a non-2xx response", async () => {
    fetchMock
      .mockResolvedValueOnce(response(500))
      .mockResolvedValueOnce(response(204));

    recordSemanticTelemetryEvent(semanticEvent("event-2"));

    await vi.advanceTimersByTimeAsync(1_000);
    expect(fetchMock).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(1_000);
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(postedBatchAt(1).events.map((event) => event.event_id)).toEqual(["event-2"]);
  });

  it("removes queued remote events when analytics is disabled but keeps local-only events", async () => {
    fetchMock.mockResolvedValue(response(204));

    recordSemanticTelemetryEvent(semanticEvent("remote-event"));
    setSemanticTelemetryRemoteEnabled(false);

    await vi.advanceTimersByTimeAsync(1_000);
    expect(fetchMock).not.toHaveBeenCalled();

    recordSemanticTelemetryEvent(semanticEvent("local-event", { delivery: "local_only" }));

    await vi.advanceTimersByTimeAsync(1_000);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(postedBatchAt(0).events.map((event) => event.event_id)).toEqual(["local-event"]);
  });
});
