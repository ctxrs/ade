// @vitest-environment jsdom

import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { PretextVirtualizerListMethods } from "@pretext-virtualizer/interface";
import { PRETEXT_VIRTUALIZER_INITIAL_BOTTOM_LOCATION } from "../state/pretextVirtualizerViewportState";
import { usePretextVirtualizerSessionController } from "./usePretextVirtualizerSessionController";
import type { WorkbenchListItem } from "./SessionPage.types";
import type { WorkbenchMessageListContext } from "./SessionPage.thread";

vi.mock("./useSessionMessageListDiagnostics", () => ({
  useSessionMessageListDiagnostics: () => ({
    recordDebugSnapshot: vi.fn(),
  }),
}));

const listItems: WorkbenchListItem[] = [
  {
    kind: "message",
    id: "message-1",
    role: "user",
    content: "one",
    attachments: [],
    created_at: "2026-03-18T00:00:00Z",
  },
  {
    kind: "message",
    id: "message-2",
    role: "assistant",
    content: "two",
    attachments: [],
    created_at: "2026-03-18T00:01:00Z",
  },
];

describe("usePretextVirtualizerSessionController", () => {
  it("always opens at the shared bottom location", () => {
    const { result } = renderHook(() =>
      usePretextVirtualizerSessionController({
        sessionId: "session-1",
        isActive: true,
        loaded: true,
        listItems,
        canLoadOlder: false,
        loadOlder: vi.fn(async () => {}),
        showDebug: false,
      }),
    );

    expect(result.current.initialLocation).toEqual(PRETEXT_VIRTUALIZER_INITIAL_BOTTOM_LOCATION);
  });

  it("requests older history only when the active user reaches the top edge while detached from bottom", async () => {
    const loadOlder = vi.fn(async () => {});
    const { result } = renderHook(() =>
      usePretextVirtualizerSessionController({
        sessionId: "session-1",
        isActive: true,
        loaded: true,
        listItems,
        canLoadOlder: true,
        loadOlder,
        showDebug: false,
      }),
    );

    await act(async () => {
      result.current.onScroll({
        listOffset: -40,
        visibleListHeight: 600,
        bottomOffset: 440,
      });
      result.current.onScroll({
        listOffset: -4,
        visibleListHeight: 600,
        bottomOffset: 360,
      });
      await Promise.resolve();
    });

    expect(loadOlder).toHaveBeenCalledTimes(1);
  });

  it("does not request older history on a top-edge downward settle", async () => {
    const loadOlder = vi.fn(async () => {});
    const { result } = renderHook(() =>
      usePretextVirtualizerSessionController({
        sessionId: "session-1",
        isActive: true,
        loaded: true,
        listItems,
        canLoadOlder: true,
        loadOlder,
        showDebug: false,
      }),
    );

    await act(async () => {
      result.current.onScroll({
        listOffset: -4,
        visibleListHeight: 600,
        bottomOffset: 440,
      });
      result.current.onScroll({
        listOffset: -20,
        visibleListHeight: 600,
        bottomOffset: 360,
      });
      await Promise.resolve();
    });

    expect(loadOlder).not.toHaveBeenCalled();
  });

  it("reopens at bottom when the same pane reactivates", () => {
    const onAtBottomChange = vi.fn();
    const scrollToBottom = vi.fn();
    const methods: PretextVirtualizerListMethods<WorkbenchListItem, WorkbenchMessageListContext> = {
      cancelSmoothScroll: () => undefined,
      scrollerElement: () => null,
      restoreAnchor: () => undefined,
      scrollToBottom,
      scrollToOffset: () => undefined,
      scrollToItem: () => undefined,
    };
    const { result, rerender } = renderHook(
      ({ isActive }: { isActive: boolean }) =>
        usePretextVirtualizerSessionController({
          sessionId: "session-1",
          isActive,
          loaded: true,
          listItems,
          canLoadOlder: false,
          loadOlder: vi.fn(async () => {}),
          showDebug: false,
          onAtBottomChange,
        }),
      { initialProps: { isActive: false } },
    );

    result.current.methodsRef.current = methods;
    rerender({ isActive: true });

    expect(scrollToBottom).toHaveBeenCalledWith("auto");
    expect(onAtBottomChange).toHaveBeenCalledWith(true);
  });

  it("continues loading older history after a prepend when the scroller remains top-pinned", async () => {
    const loadOlder = vi.fn(async () => {});
    const scroller = document.createElement("div");
    Object.defineProperty(scroller, "scrollTop", { value: 0, writable: true });
    Object.defineProperty(scroller, "scrollHeight", { value: 3000, configurable: true });
    Object.defineProperty(scroller, "clientHeight", { value: 600, configurable: true });
    const methods: PretextVirtualizerListMethods<WorkbenchListItem, WorkbenchMessageListContext> = {
      cancelSmoothScroll: () => undefined,
      scrollerElement: () => scroller,
      restoreAnchor: () => undefined,
      scrollToBottom: () => undefined,
      scrollToOffset: () => undefined,
      scrollToItem: () => undefined,
    };
    const extendedListItems: WorkbenchListItem[] = [
      {
        kind: "message",
        id: "message-0",
        role: "user",
        content: "older",
        attachments: [],
        created_at: "2026-03-17T23:59:00Z",
      },
      ...listItems,
    ];

    const { result, rerender } = renderHook(
      ({ items }: { items: WorkbenchListItem[] }) =>
        usePretextVirtualizerSessionController({
          sessionId: "session-1",
          isActive: true,
          loaded: true,
          listItems: items,
          canLoadOlder: true,
          loadOlder,
          showDebug: false,
        }),
      { initialProps: { items: listItems } },
    );

    result.current.methodsRef.current = methods;

    await act(async () => {
      result.current.onScroll({
        listOffset: -40,
        visibleListHeight: 600,
        bottomOffset: 440,
      });
      result.current.onScroll({
        listOffset: -4,
        visibleListHeight: 600,
        bottomOffset: 360,
      });
      await Promise.resolve();
    });

    expect(loadOlder).toHaveBeenCalledTimes(1);

    act(() => {
      rerender({ items: extendedListItems });
    });

    await waitFor(() => {
      expect(loadOlder).toHaveBeenCalledTimes(2);
    });
  });

  it("marks the initial projection as rendered once the first visible range arrives", () => {
    const onInitialContentRendered = vi.fn();
    const { result } = renderHook(() =>
      usePretextVirtualizerSessionController({
        sessionId: "session-1",
        isActive: true,
        loaded: true,
        listItems,
        canLoadOlder: false,
        loadOlder: vi.fn(async () => {}),
        showDebug: false,
        onInitialContentRendered,
      }),
    );

    act(() => {
      result.current.onRenderedDataChange([listItems[0], listItems[1]]);
      result.current.onRenderedDataChange([listItems[0]]);
    });

    expect(onInitialContentRendered).toHaveBeenCalledTimes(1);
  });

  it("does not request older history while still well below the top edge", async () => {
    const loadOlder = vi.fn(async () => {});
    const { result } = renderHook(() =>
      usePretextVirtualizerSessionController({
        sessionId: "session-1",
        isActive: true,
        loaded: true,
        listItems,
        canLoadOlder: true,
        loadOlder,
        showDebug: false,
      }),
    );

    await act(async () => {
      result.current.onScroll({
        listOffset: -120,
        visibleListHeight: 600,
        bottomOffset: 540,
      });
      result.current.onScroll({
        listOffset: -20,
        visibleListHeight: 600,
        bottomOffset: 440,
      });
      await Promise.resolve();
    });

    expect(loadOlder).not.toHaveBeenCalled();
  });
});
