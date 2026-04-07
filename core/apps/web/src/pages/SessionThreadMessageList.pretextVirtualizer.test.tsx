// @vitest-environment jsdom

import { act, fireEvent, render, screen } from "@testing-library/react";
import { createRef, useEffect } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PretextVirtualizerListMethods } from "@pretext-virtualizer/interface";
import { SessionThreadPretextVirtualizerList } from "./SessionThreadMessageList.pretextVirtualizer";
import type { WorkbenchListItem } from "./SessionPage.types";
import type { WorkbenchMessageListContext } from "./SessionPage.thread";
import type { WorkbenchThreadProjectionOp } from "./sessionThreadProjection";
import { getOrCreateSessionPretextRuntime, resetSessionPretextRuntimeCache } from "./sessionThread/pretextSessionRuntimeCache";

const resizeObserverInstances: Array<{ callback: ResizeObserverCallback }> = [];

class ResizeObserverStub {
  callback: ResizeObserverCallback;

  constructor(callback: ResizeObserverCallback) {
    this.callback = callback;
    resizeObserverInstances.push({ callback });
  }

  observe(): void {}

  disconnect(): void {}
}

const context: WorkbenchMessageListContext = {
  loaded: true,
  loadingOlder: false,
  expandedTurnHeaders: {},
  expandedTurnDetailsById: {},
  expandedToolById: {},
  expandedMessageById: {},
  turnToolsLoading: [],
  verbosity: "default",
};

const makeItems = (count = 2): WorkbenchListItem[] =>
  Array.from({ length: count }, (_, index) => ({
    kind: "message" as const,
    id: `message-${index + 1}`,
    role: index % 2 === 0 ? ("user" as const) : ("assistant" as const),
    content: `message ${index + 1}`,
    attachments: [],
    created_at: `2026-03-17T00:0${index}:00Z`,
  }));

const noopProjectionOp: WorkbenchThreadProjectionOp = {
  kind: "noop",
  projectionRevision: 0,
  changedItemIds: [],
  remeasureItemIds: [],
};

function defineScrollerMetrics(scroller: HTMLElement, metrics: { clientHeight: number; clientWidth: number; scrollHeight: number }) {
  Object.defineProperty(scroller, "clientHeight", { configurable: true, value: metrics.clientHeight });
  Object.defineProperty(scroller, "clientWidth", { configurable: true, value: metrics.clientWidth });
  Object.defineProperty(scroller, "scrollHeight", { configurable: true, value: metrics.scrollHeight, writable: true });
}

describe("SessionThreadPretextVirtualizerList", () => {
  beforeEach(() => {
    resizeObserverInstances.length = 0;
    resetSessionPretextRuntimeCache();
    Object.defineProperty(globalThis, "ResizeObserver", {
      configurable: true,
      value: ResizeObserverStub,
    });
  });

  it("renders a deterministic transcript surface and exposes imperative methods", () => {
    const methodsRef = createRef<PretextVirtualizerListMethods<WorkbenchListItem, WorkbenchMessageListContext> | null>();

    const { container } = render(
      <SessionThreadPretextVirtualizerList
        style={{ height: 400 }}
        sessionId="session-1"
        isActive
        listItems={makeItems()}
        threadProjectionOp={noopProjectionOp}
        itemContent={(_, item) => <div>{item.id}</div>}
        itemKey={(item) => item.id}
        context={context}
        methodsRef={methodsRef}
      />,
    );

    const scroller = container.querySelector<HTMLElement>("[data-pretext-virtualizer-list='1']");
    expect(scroller).not.toBeNull();
    expect(methodsRef.current).not.toBeNull();
    expect(typeof methodsRef.current?.scrollToBottom).toBe("function");
    expect(container.querySelectorAll("[data-pretext-virtualizer-row='1']").length).toBeGreaterThan(0);
  });

  it("shows the jump-to-latest control when detached from bottom", () => {
    const { container } = render(
      <SessionThreadPretextVirtualizerList
        style={{ height: 400 }}
        sessionId="session-1"
        isActive
        listItems={makeItems(20)}
        threadProjectionOp={noopProjectionOp}
        itemContent={(_, item) => <div>{item.id}</div>}
        itemKey={(item) => item.id}
        context={context}
      />,
    );

    const scroller = container.querySelector<HTMLElement>("[data-pretext-virtualizer-list='1']");
    if (!scroller) throw new Error("Expected transcript scroller");
    defineScrollerMetrics(scroller, { clientHeight: 300, clientWidth: 900, scrollHeight: 1400 });

    act(() => {
      resizeObserverInstances[0]?.callback([], {} as ResizeObserver);
      scroller.scrollTop = 900;
      fireEvent.scroll(scroller);
      fireEvent.wheel(scroller, { deltaY: -120 });
      scroller.scrollTop = 420;
      fireEvent.scroll(scroller);
    });

    expect(screen.getByRole("button", { name: "Jump to latest" })).toBeInTheDocument();
  });

  it("does not snap back to bottom after appending while detached", () => {
    const { container, rerender } = render(
      <SessionThreadPretextVirtualizerList
        style={{ height: 400 }}
        sessionId="session-1"
        isActive
        listItems={makeItems(20)}
        threadProjectionOp={noopProjectionOp}
        itemContent={(_, item) => <div>{item.id}</div>}
        itemKey={(item) => item.id}
        context={context}
      />,
    );

    const scroller = container.querySelector<HTMLElement>("[data-pretext-virtualizer-list='1']");
    if (!scroller) throw new Error("Expected transcript scroller");
    defineScrollerMetrics(scroller, { clientHeight: 300, clientWidth: 900, scrollHeight: 1400 });

    act(() => {
      resizeObserverInstances[0]?.callback([], {} as ResizeObserver);
      scroller.scrollTop = 920;
      fireEvent.scroll(scroller);
      fireEvent.wheel(scroller, { deltaY: -120 });
      scroller.scrollTop = 500;
      fireEvent.scroll(scroller);
    });

    const detachedScrollTop = scroller.scrollTop;

    rerender(
      <SessionThreadPretextVirtualizerList
        style={{ height: 400 }}
        sessionId="session-1"
        isActive
        listItems={makeItems(21)}
        threadProjectionOp={{ ...noopProjectionOp, kind: "append_stream", changedItemIds: ["message-21"], remeasureItemIds: ["message-20", "message-21"] }}
        itemContent={(_, item) => <div>{item.id}</div>}
        itemKey={(item) => item.id}
        context={context}
      />,
    );

    expect(scroller.scrollTop).toBeLessThan(900);
    expect(Math.abs(scroller.scrollTop - detachedScrollTop)).toBeLessThan(80);
  });

  it("detaches from bottom on direct scroll without wheel and stays detached on append", () => {
    const { container, rerender } = render(
      <SessionThreadPretextVirtualizerList
        style={{ height: 400 }}
        sessionId="session-1"
        isActive
        listItems={makeItems(20)}
        threadProjectionOp={noopProjectionOp}
        itemContent={(_, item) => <div>{item.id}</div>}
        itemKey={(item) => item.id}
        context={context}
      />,
    );

    const scroller = container.querySelector<HTMLElement>("[data-pretext-virtualizer-list='1']");
    if (!scroller) throw new Error("Expected transcript scroller");
    defineScrollerMetrics(scroller, { clientHeight: 300, clientWidth: 900, scrollHeight: 1400 });

    act(() => {
      resizeObserverInstances[0]?.callback([], {} as ResizeObserver);
      scroller.scrollTop = 920;
      fireEvent.scroll(scroller);
      scroller.scrollTop = 500;
      fireEvent.scroll(scroller);
    });

    const detachedScrollTop = scroller.scrollTop;

    rerender(
      <SessionThreadPretextVirtualizerList
        style={{ height: 400 }}
        sessionId="session-1"
        isActive
        listItems={makeItems(21)}
        threadProjectionOp={{ ...noopProjectionOp, kind: "append_stream", changedItemIds: ["message-21"], remeasureItemIds: ["message-20", "message-21"] }}
        itemContent={(_, item) => <div>{item.id}</div>}
        itemKey={(item) => item.id}
        context={context}
      />,
    );

    expect(Math.abs(scroller.scrollTop - detachedScrollTop)).toBeLessThan(80);
  });

  it("does not remount stable rows when appending new rows", () => {
    const mounts = new Map<string, number>();

    function Row({ item }: { item: WorkbenchListItem }) {
      useEffect(() => {
        mounts.set(item.id, (mounts.get(item.id) ?? 0) + 1);
      }, [item.id]);
      return <div>{item.id}</div>;
    }

    const { rerender } = render(
      <SessionThreadPretextVirtualizerList
        style={{ height: 400 }}
        sessionId="session-1"
        isActive
        listItems={makeItems(3)}
        threadProjectionOp={noopProjectionOp}
        itemContent={(_, item) => <Row item={item} />}
        itemKey={(item) => item.id}
        context={context}
      />,
    );

    expect(mounts.get("message-1")).toBe(1);
    expect(mounts.get("message-2")).toBe(1);
    expect(mounts.get("message-3")).toBe(1);

    rerender(
      <SessionThreadPretextVirtualizerList
        style={{ height: 400 }}
        sessionId="session-1"
        isActive
        listItems={makeItems(4)}
        threadProjectionOp={{ ...noopProjectionOp, kind: "append_stream", changedItemIds: ["message-4"], remeasureItemIds: ["message-3", "message-4"] }}
        itemContent={(_, item) => <Row item={item} />}
        itemKey={(item) => item.id}
        context={context}
      />,
    );

    expect(mounts.get("message-1")).toBe(1);
    expect(mounts.get("message-2")).toBe(1);
    expect(mounts.get("message-3")).toBe(1);
    expect(mounts.get("message-4")).toBe(1);
  });

  it("keeps the viewport pinned to bottom when the viewport height shrinks while attached", () => {
    const { container } = render(
      <SessionThreadPretextVirtualizerList
        style={{ height: 400 }}
        sessionId="session-1"
        isActive
        listItems={makeItems(24)}
        threadProjectionOp={noopProjectionOp}
        itemContent={(_, item) => <div style={{ height: 48 }}>{item.id}</div>}
        itemKey={(item) => item.id}
        context={context}
      />,
    );

    const scroller = container.querySelector<HTMLElement>("[data-pretext-virtualizer-list='1']");
    if (!scroller) throw new Error("Expected transcript scroller");

    defineScrollerMetrics(scroller, { clientHeight: 400, clientWidth: 900, scrollHeight: 1800 });
    act(() => {
      resizeObserverInstances[0]?.callback([], {} as ResizeObserver);
    });

    const beforeResizeScrollTop = scroller.scrollTop;

    defineScrollerMetrics(scroller, { clientHeight: 300, clientWidth: 900, scrollHeight: 1800 });
    act(() => {
      resizeObserverInstances[0]?.callback([], {} as ResizeObserver);
    });

    expect(scroller.scrollTop).toBeGreaterThan(beforeResizeScrollTop);
  });

  it("resyncs row offsets when only layout context changes", () => {
    const expandableContent = Array.from(
      { length: 28 },
      (_, index) => `- expanded row ${index + 1} with enough text to wrap and change the measured height`,
    ).join("\n");
    const listItems: WorkbenchListItem[] = [
      {
        kind: "message",
        id: "message-expandable",
        role: "user",
        content: expandableContent,
        attachments: [],
        created_at: "2026-04-06T00:00:00Z",
      },
      {
        kind: "turn_status",
        id: "status-1",
        turn_id: "turn-1",
        created_at: "2026-04-06T00:01:00Z",
        started_at: "2026-04-06T00:01:00Z",
        updated_at: "2026-04-06T00:01:05Z",
        status: "completed",
        assistant_messages_content: "done",
      },
    ];

    const { container, rerender } = render(
      <SessionThreadPretextVirtualizerList
        style={{ height: 400 }}
        sessionId="session-1"
        isActive
        listItems={listItems}
        threadProjectionOp={noopProjectionOp}
        itemContent={(_, item) => <div>{item.id}</div>}
        itemKey={(item) => item.id}
        context={{ ...context, expandedMessageById: {} }}
      />,
    );

    const scroller = container.querySelector<HTMLElement>("[data-pretext-virtualizer-list='1']");
    if (!scroller) throw new Error("Expected transcript scroller");
    defineScrollerMetrics(scroller, { clientHeight: 300, clientWidth: 900, scrollHeight: 1400 });

    act(() => {
      resizeObserverInstances[0]?.callback([], {} as ResizeObserver);
    });

    const shellsBefore = container.querySelectorAll<HTMLElement>("[data-pretext-virtualizer-row-shell='1']");
    const secondTopBefore = Number.parseFloat(shellsBefore[1]?.style.top ?? "0");

    rerender(
      <SessionThreadPretextVirtualizerList
        style={{ height: 400 }}
        sessionId="session-1"
        isActive
        listItems={listItems}
        threadProjectionOp={{
          kind: "toggle_expansion",
          projectionRevision: 1,
          changedItemIds: ["message-expandable"],
          remeasureItemIds: ["message-expandable", "status-1"],
        }}
        itemContent={(_, item) => <div>{item.id}</div>}
        itemKey={(item) => item.id}
        context={{ ...context, expandedMessageById: { "message-expandable": true } }}
      />,
    );

    act(() => {
      resizeObserverInstances[0]?.callback([], {} as ResizeObserver);
    });

    const shellsAfter = container.querySelectorAll<HTMLElement>("[data-pretext-virtualizer-row-shell='1']");
    const secondTopAfter = Number.parseFloat(shellsAfter[1]?.style.top ?? "0");

    expect(secondTopAfter).toBeGreaterThan(secondTopBefore);
  });

  it("does not re-enter projection sync during ordinary scroll with unchanged items", () => {
    const sessionId = "session-scroll";
    const { container } = render(
      <SessionThreadPretextVirtualizerList
        style={{ height: 400 }}
        sessionId={sessionId}
        isActive
        listItems={makeItems(20)}
        threadProjectionOp={noopProjectionOp}
        itemContent={(_, item) => <div>{item.id}</div>}
        itemKey={(item) => item.id}
        context={context}
      />,
    );

    const scroller = container.querySelector<HTMLElement>("[data-pretext-virtualizer-list='1']");
    if (!scroller) throw new Error("Expected transcript scroller");
    defineScrollerMetrics(scroller, { clientHeight: 300, clientWidth: 900, scrollHeight: 1400 });

    act(() => {
      resizeObserverInstances[0]?.callback([], {} as ResizeObserver);
    });

    const runtime = getOrCreateSessionPretextRuntime(sessionId);
    const syncViewportSpy = vi.spyOn(runtime.core, "syncViewport");
    const syncItemsSpy = vi.spyOn(runtime.core, "syncItems");
    syncViewportSpy.mockClear();
    syncItemsSpy.mockClear();

    act(() => {
      scroller.scrollTop = 500;
      fireEvent.scroll(scroller);
    });

    expect(syncViewportSpy).toHaveBeenCalledTimes(1);
    expect(syncItemsSpy).not.toHaveBeenCalled();
  });

  it("replans offsets when context changes even if the projection op is noop", () => {
    const expandableContent = Array.from(
      { length: 28 },
      (_, index) => `- expanded row ${index + 1} with enough text to wrap and change the measured height`,
    ).join("\n");
    const listItems: WorkbenchListItem[] = [
      {
        kind: "message",
        id: "message-expandable-noop",
        role: "user",
        content: expandableContent,
        attachments: [],
        created_at: "2026-04-06T00:00:00Z",
      },
      {
        kind: "turn_status",
        id: "status-2",
        turn_id: "turn-2",
        created_at: "2026-04-06T00:01:00Z",
        started_at: "2026-04-06T00:01:00Z",
        updated_at: "2026-04-06T00:01:05Z",
        status: "completed",
        assistant_messages_content: "done",
      },
    ];

    const { container, rerender } = render(
      <SessionThreadPretextVirtualizerList
        style={{ height: 400 }}
        sessionId="session-noop-layout"
        isActive
        listItems={listItems}
        threadProjectionOp={noopProjectionOp}
        itemContent={(_, item) => <div>{item.id}</div>}
        itemKey={(item) => item.id}
        context={{ ...context, expandedMessageById: {} }}
      />,
    );

    const scroller = container.querySelector<HTMLElement>("[data-pretext-virtualizer-list='1']");
    if (!scroller) throw new Error("Expected transcript scroller");
    defineScrollerMetrics(scroller, { clientHeight: 300, clientWidth: 900, scrollHeight: 1400 });

    act(() => {
      resizeObserverInstances[0]?.callback([], {} as ResizeObserver);
    });

    const shellsBefore = container.querySelectorAll<HTMLElement>("[data-pretext-virtualizer-row-shell='1']");
    const secondTopBefore = Number.parseFloat(shellsBefore[1]?.style.top ?? "0");

    rerender(
      <SessionThreadPretextVirtualizerList
        style={{ height: 400 }}
        sessionId="session-noop-layout"
        isActive
        listItems={listItems}
        threadProjectionOp={noopProjectionOp}
        itemContent={(_, item) => <div>{item.id}</div>}
        itemKey={(item) => item.id}
        context={{ ...context, expandedMessageById: { "message-expandable-noop": true } }}
      />,
    );

    act(() => {
      resizeObserverInstances[0]?.callback([], {} as ResizeObserver);
    });

    const shellsAfter = container.querySelectorAll<HTMLElement>("[data-pretext-virtualizer-row-shell='1']");
    const secondTopAfter = Number.parseFloat(shellsAfter[1]?.style.top ?? "0");

    expect(secondTopAfter).toBeGreaterThan(secondTopBefore);
  });
});
