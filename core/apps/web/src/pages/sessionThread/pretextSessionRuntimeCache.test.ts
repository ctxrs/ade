import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkbenchListItem } from "../SessionPage.types";
import {
  createDefaultSessionTranscriptUiState,
  getOrCreateSessionPretextRuntime,
  SESSION_PRETEXT_MAX_RESTORABLE_RUNTIMES,
  getSessionPretextRuntimeCacheSize,
  markSessionPretextRuntimeVisible,
  noteSessionPretextRuntimeSnapshot,
  primeSessionPretextRuntime,
  pruneSessionPretextRuntimeCache,
  readSessionPretextRuntimeRestoreSnapshot,
  resetSessionPretextRuntimeCache,
} from "./pretextSessionRuntimeCache";

const makeItems = (count = 2): WorkbenchListItem[] =>
  Array.from({ length: count }, (_, index) => ({
    kind: "message" as const,
    id: `message-${index + 1}`,
    role: index % 2 === 0 ? ("user" as const) : ("assistant" as const),
    content: `message ${index + 1}`,
    attachments: [],
    created_at: `2026-03-17T00:${String(index).padStart(2, "0")}:00Z`,
  }));

describe("pretextSessionRuntimeCache", () => {
  beforeEach(() => {
    resetSessionPretextRuntimeCache();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("preserves detached restore state when priming newer items in the background", () => {
    const sessionId = "session-detached";
    const initialItems = makeItems(10);
    const updatedItems = makeItems(12);
    const runtime = getOrCreateSessionPretextRuntime(sessionId, {
      uiState: createDefaultSessionTranscriptUiState(),
    });

    runtime.core.replaceItems(initialItems, { kind: "bottom" });
    markSessionPretextRuntimeVisible(runtime, true);
    const detachedSnapshot = runtime.core.syncViewport({
      width: 900,
      height: 300,
      scrollTop: 420,
    });
    noteSessionPretextRuntimeSnapshot(runtime, detachedSnapshot, initialItems);
    markSessionPretextRuntimeVisible(runtime, false);

    const restoreBefore = readSessionPretextRuntimeRestoreSnapshot(runtime);
    primeSessionPretextRuntime({
      sessionId,
      listItems: updatedItems,
      uiState: createDefaultSessionTranscriptUiState(),
      viewportWidth: 900,
      viewportHeight: 300,
    });
    const restoreAfter = readSessionPretextRuntimeRestoreSnapshot(runtime);

    expect(restoreBefore).not.toBeNull();
    expect(restoreAfter).not.toBeNull();
    expect(restoreAfter?.anchor).toEqual(restoreBefore?.anchor);
    expect(restoreAfter?.scrollTop).toBe(restoreBefore?.scrollTop);
  });

  it("skips item replacement when the ui state is semantically unchanged", () => {
    const listItems = makeItems(8);
    const runtime = primeSessionPretextRuntime({
      sessionId: "session-stable-ui",
      listItems,
      uiState: createDefaultSessionTranscriptUiState("default", ["turn-1"]),
      viewportWidth: 900,
      viewportHeight: 300,
    });
    const replaceItemsSpy = vi.spyOn(runtime.core, "replaceItems");

    primeSessionPretextRuntime({
      sessionId: "session-stable-ui",
      listItems,
      uiState: createDefaultSessionTranscriptUiState("default", ["turn-1"]),
      viewportWidth: 900,
      viewportHeight: 300,
    });

    expect(replaceItemsSpy).not.toHaveBeenCalled();
  });

  it("evicts cold prepared runtimes while retaining detached restore state", () => {
    const restorableRuntime = getOrCreateSessionPretextRuntime("session-restorable", {
      uiState: createDefaultSessionTranscriptUiState(),
    });
    const restorableItems = makeItems(6);

    restorableRuntime.core.replaceItems(restorableItems, { kind: "bottom" });
    markSessionPretextRuntimeVisible(restorableRuntime, true);
    noteSessionPretextRuntimeSnapshot(
      restorableRuntime,
      restorableRuntime.core.syncViewport({
        width: 900,
        height: 300,
        scrollTop: 360,
      }),
      restorableItems,
    );
    markSessionPretextRuntimeVisible(restorableRuntime, false);

    primeSessionPretextRuntime({
      sessionId: "session-retained",
      listItems: makeItems(4),
      uiState: createDefaultSessionTranscriptUiState(),
      viewportWidth: 900,
      viewportHeight: 300,
    });
    primeSessionPretextRuntime({
      sessionId: "session-evicted",
      listItems: makeItems(5),
      uiState: createDefaultSessionTranscriptUiState(),
      viewportWidth: 900,
      viewportHeight: 300,
    });

    expect(getSessionPretextRuntimeCacheSize()).toBe(3);

    pruneSessionPretextRuntimeCache(["session-retained"]);

    expect(getSessionPretextRuntimeCacheSize()).toBe(2);
    expect(readSessionPretextRuntimeRestoreSnapshot(getOrCreateSessionPretextRuntime("session-restorable"))).not.toBeNull();
    expect(readSessionPretextRuntimeRestoreSnapshot(getOrCreateSessionPretextRuntime("session-retained"))).toBeNull();
    expect(getSessionPretextRuntimeCacheSize()).toBe(2);
  });

  it("bounds detached restore entries by recency", () => {
    vi.useFakeTimers();
    const width = 900;
    const height = 300;

    for (let index = 0; index < SESSION_PRETEXT_MAX_RESTORABLE_RUNTIMES + 3; index += 1) {
      vi.setSystemTime(new Date(`2026-03-17T00:${String(index).padStart(2, "0")}:00Z`));
      const sessionId = `session-restorable-${index}`;
      const runtime = getOrCreateSessionPretextRuntime(sessionId, {
        uiState: createDefaultSessionTranscriptUiState(),
      });
      const items = makeItems(index + 1);
      runtime.core.replaceItems(items, { kind: "bottom" });
      markSessionPretextRuntimeVisible(runtime, true);
      noteSessionPretextRuntimeSnapshot(
        runtime,
        runtime.core.syncViewport({
          width,
          height,
          scrollTop: 120 + index,
        }),
        items,
      );
      markSessionPretextRuntimeVisible(runtime, false);
    }

    pruneSessionPretextRuntimeCache([]);

    expect(getSessionPretextRuntimeCacheSize()).toBe(SESSION_PRETEXT_MAX_RESTORABLE_RUNTIMES);
    expect(
      readSessionPretextRuntimeRestoreSnapshot(getOrCreateSessionPretextRuntime("session-restorable-0")),
    ).toBeNull();
    expect(
      readSessionPretextRuntimeRestoreSnapshot(
        getOrCreateSessionPretextRuntime(
          `session-restorable-${SESSION_PRETEXT_MAX_RESTORABLE_RUNTIMES + 2}`,
        ),
      ),
    ).not.toBeNull();
  });
});
