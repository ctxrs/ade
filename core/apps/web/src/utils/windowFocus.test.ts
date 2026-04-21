import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const setDocumentForeground = ({
  focused,
  visibility,
}: {
  focused: boolean;
  visibility: DocumentVisibilityState;
}) => {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: visibility,
  });
  vi.spyOn(document, "hasFocus").mockReturnValue(focused);
};

describe("windowFocus", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.resetModules();
    vi.useRealTimers();
    window.localStorage.clear();
    setDocumentForeground({ focused: true, visibility: "visible" });
  });

  afterEach(() => {
    vi.restoreAllMocks();
    window.localStorage.clear();
  });

  it("records foreground ownership for the focused window", async () => {
    const { APP_FOREGROUND_STORAGE_KEY, isAppInForeground } = await import("./windowFocus");

    expect(isAppInForeground()).toBe(true);

    const raw = window.localStorage.getItem(APP_FOREGROUND_STORAGE_KEY);
    expect(raw).toBeTruthy();
    expect(JSON.parse(String(raw))).toEqual(
      expect.objectContaining({
        updatedAtMs: expect.any(Number),
        windowId: expect.any(String),
      }),
    );
  });

  it("initializes the shared foreground marker without waiting for a notification check", async () => {
    const { APP_FOREGROUND_STORAGE_KEY, initializeAppForegroundTracking } = await import(
      "./windowFocus"
    );

    initializeAppForegroundTracking();

    expect(window.localStorage.getItem(APP_FOREGROUND_STORAGE_KEY)).toBeTruthy();
  });

  it("treats another focused window marker as app foreground", async () => {
    setDocumentForeground({ focused: false, visibility: "hidden" });
    const { APP_FOREGROUND_STORAGE_KEY, isAppInForeground } = await import("./windowFocus");
    window.localStorage.setItem(
      APP_FOREGROUND_STORAGE_KEY,
      JSON.stringify({
        windowId: "other-window",
        updatedAtMs: Date.now(),
      }),
    );

    expect(isAppInForeground()).toBe(true);
  });

  it("drops stale foreground markers", async () => {
    setDocumentForeground({ focused: false, visibility: "hidden" });
    const { APP_FOREGROUND_STORAGE_KEY, isAppInForeground } = await import("./windowFocus");
    window.localStorage.setItem(
      APP_FOREGROUND_STORAGE_KEY,
      JSON.stringify({
        windowId: "other-window",
        updatedAtMs: Date.now() - 30_000,
      }),
    );

    expect(isAppInForeground()).toBe(false);
    expect(window.localStorage.getItem(APP_FOREGROUND_STORAGE_KEY)).toBeNull();
  });

  it("clears this window's marker after blur when no other ctx window is focused", async () => {
    const { APP_FOREGROUND_STORAGE_KEY, isAppInForeground } = await import("./windowFocus");

    expect(isAppInForeground()).toBe(true);
    setDocumentForeground({ focused: false, visibility: "hidden" });
    window.dispatchEvent(new Event("blur"));

    expect(isAppInForeground()).toBe(false);
    expect(window.localStorage.getItem(APP_FOREGROUND_STORAGE_KEY)).toBeNull();
  });

  it("refreshes the shared foreground marker while this window remains focused", async () => {
    vi.useFakeTimers();
    const { APP_FOREGROUND_STORAGE_KEY, isAppInForeground } = await import("./windowFocus");

    expect(isAppInForeground()).toBe(true);
    const initialMarker = JSON.parse(
      String(window.localStorage.getItem(APP_FOREGROUND_STORAGE_KEY)),
    ) as { updatedAtMs: number };

    vi.advanceTimersByTime(10_000);

    const refreshedMarker = JSON.parse(
      String(window.localStorage.getItem(APP_FOREGROUND_STORAGE_KEY)),
    ) as { updatedAtMs: number };
    expect(refreshedMarker.updatedAtMs).toBeGreaterThan(initialMarker.updatedAtMs);
  });
});
