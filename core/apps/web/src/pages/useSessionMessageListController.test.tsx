import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionMessageListController } from "./useSessionMessageListController";
import type { WorkbenchListItem } from "./SessionPage.types";

let coalescedItems: WorkbenchListItem[] = [];

vi.mock("../components/hooks/useRafCoalesced", () => ({
  useRafCoalesced: () => coalescedItems,
}));

const makeSpacer = (id: string): WorkbenchListItem => ({ id, kind: "spacer", created_at: "2026-03-18T00:00:00.000Z" });

function createFakeMethods(initialItems: WorkbenchListItem[] = []) {
  let items = [...initialItems];
  const replace = vi.fn((next: WorkbenchListItem[]) => {
    items = [...next];
  });
  const deleteRange = vi.fn((start: number, count: number) => {
    items = [...items.slice(0, start), ...items.slice(start + count)];
  });
  const insert = vi.fn((inserted: WorkbenchListItem[], offset: number) => {
    items = [...items.slice(0, offset), ...inserted, ...items.slice(offset)];
  });
  const append = vi.fn((suffix: WorkbenchListItem[]) => {
    items = [...items, ...suffix];
  });
  const prepend = vi.fn((prefix: WorkbenchListItem[]) => {
    items = [...prefix, ...items];
  });
  const map = vi.fn((mapper: (item: WorkbenchListItem) => WorkbenchListItem) => {
    items = items.map((item) => mapper(item));
  });
  const mapWithAnchor = vi.fn((mapper: (item: WorkbenchListItem) => WorkbenchListItem) => {
    items = items.map((item) => mapper(item));
  });
  const batch = vi.fn((updater: () => void) => {
    updater();
  });
  const cancelSmoothScroll = vi.fn();
  const scrollToItem = vi.fn();
  const scrollerElement = vi.fn(() => null);

  return {
    methods: {
      data: {
        get: () => items,
        replace,
        deleteRange,
        insert,
        append,
        prepend,
        map,
        mapWithAnchor,
        batch,
      },
      cancelSmoothScroll,
      scrollToItem,
      scrollerElement,
    },
    spies: {
      replace,
      deleteRange,
      insert,
      append,
      prepend,
      map,
      mapWithAnchor,
      batch,
      cancelSmoothScroll,
      scrollToItem,
      scrollerElement,
    },
  };
}

describe("useSessionMessageListController", () => {
  beforeEach(() => {
    coalescedItems = [];
  });

  it("initializes the keyed message list from the visible coalesced items", () => {
    const rawItems = [makeSpacer("raw-1"), makeSpacer("raw-2"), makeSpacer("raw-3")];
    coalescedItems = [makeSpacer("visible-1"), makeSpacer("visible-2")];

    const { result } = renderHook(() =>
      useSessionMessageListController({
        sessionId: "session-1",
        isActive: false,
        loaded: true,
        listItems: rawItems,
        canLoadOlder: false,
        loadOlder: async () => {},
        layoutRevision: "layout-1",
        itemSizeCacheKey: () => null,
        showDebug: false,
      }),
    );

    expect(result.current.initialData).toEqual(coalescedItems);
  });

  it("bypasses coalescing during a session boundary", () => {
    const sessionOneRaw = [makeSpacer("session-1-raw")];
    const sessionTwoRaw = [makeSpacer("session-2-raw-1"), makeSpacer("session-2-raw-2")];
    coalescedItems = [makeSpacer("session-1-visible")];

    const { result, rerender } = renderHook(
      ({ sessionId, listItems }: { sessionId: string; listItems: WorkbenchListItem[] }) =>
        useSessionMessageListController({
          sessionId,
          isActive: false,
          loaded: true,
          listItems,
          canLoadOlder: false,
          loadOlder: async () => {},
          layoutRevision: "layout-1",
          itemSizeCacheKey: () => null,
          showDebug: false,
        }),
      {
        initialProps: {
          sessionId: "session-1",
          listItems: sessionOneRaw,
        },
      },
    );

    expect(result.current.initialData).toEqual(coalescedItems);

    coalescedItems = [makeSpacer("stale-visible-session-1")];
    rerender({
      sessionId: "session-2",
      listItems: sessionTwoRaw,
    });

    expect(result.current.initialData).toEqual(sessionTwoRaw);
  });

  it("replaces bottom-locked mixed structural updates instead of reconciling them", () => {
    const initialItems = Array.from({ length: 10 }, (_, index) => makeSpacer(`current-${index}`));
    const mixedStructuralNext = [
      initialItems[0]!,
      ...Array.from({ length: 20 }, (_, index) => makeSpacer(`next-middle-${index}`)),
      initialItems.at(-1)!,
    ];
    const fake = createFakeMethods();
    coalescedItems = initialItems;

    const { result, rerender } = renderHook(
      ({ listItems, layoutRevision }: { listItems: WorkbenchListItem[]; layoutRevision: string }) =>
        useSessionMessageListController({
          sessionId: "session-1",
          isActive: true,
          loaded: true,
          listItems,
          canLoadOlder: false,
          loadOlder: async () => {},
          layoutRevision,
          itemSizeCacheKey: () => null,
          showDebug: false,
        }),
      {
        initialProps: {
          listItems: initialItems,
          layoutRevision: "layout-1",
        },
      },
    );

    result.current.methodsRef.current = fake.methods as unknown as typeof result.current.methodsRef.current;
    rerender({
      listItems: [...initialItems],
      layoutRevision: "layout-1",
    });

    fake.spies.replace.mockClear();
    fake.spies.deleteRange.mockClear();
    fake.spies.insert.mockClear();
    fake.spies.batch.mockClear();
    coalescedItems = mixedStructuralNext;

    rerender({
      listItems: mixedStructuralNext,
      layoutRevision: "layout-1",
    });

    expect(fake.spies.replace).toHaveBeenCalledTimes(1);
    expect(fake.spies.replace).toHaveBeenLastCalledWith(mixedStructuralNext, {
      initialLocation: { index: "LAST", align: "end" },
      purgeItemSizes: true,
    });
    expect(fake.spies.deleteRange).not.toHaveBeenCalled();
    expect(fake.spies.insert).not.toHaveBeenCalled();
    expect(fake.spies.batch).not.toHaveBeenCalled();
  });

  it("replaces same-length large middle churn while bottom-locked", () => {
    const initialItems = Array.from({ length: 231 }, (_, index) => makeSpacer(`current-${index}`));
    const mixedStructuralNext = [
      ...initialItems.slice(0, 92),
      ...Array.from({ length: 138 }, (_, index) => makeSpacer(`next-middle-${index}`)),
      initialItems.at(-1)!,
    ];
    const fake = createFakeMethods();
    coalescedItems = initialItems;

    const { result, rerender } = renderHook(
      ({ listItems, layoutRevision }: { listItems: WorkbenchListItem[]; layoutRevision: string }) =>
        useSessionMessageListController({
          sessionId: "session-1",
          isActive: true,
          loaded: true,
          listItems,
          canLoadOlder: false,
          loadOlder: async () => {},
          layoutRevision,
          itemSizeCacheKey: () => null,
          showDebug: false,
        }),
      {
        initialProps: {
          listItems: initialItems,
          layoutRevision: "layout-1",
        },
      },
    );

    result.current.methodsRef.current = fake.methods as unknown as typeof result.current.methodsRef.current;
    rerender({
      listItems: [...initialItems],
      layoutRevision: "layout-1",
    });

    fake.spies.replace.mockClear();
    fake.spies.deleteRange.mockClear();
    fake.spies.insert.mockClear();
    fake.spies.batch.mockClear();
    coalescedItems = mixedStructuralNext;

    rerender({
      listItems: mixedStructuralNext,
      layoutRevision: "layout-1",
    });

    expect(fake.spies.replace).toHaveBeenCalledTimes(1);
    expect(fake.spies.replace).toHaveBeenLastCalledWith(mixedStructuralNext, {
      initialLocation: { index: "LAST", align: "end" },
      purgeItemSizes: true,
    });
    expect(fake.spies.deleteRange).not.toHaveBeenCalled();
    expect(fake.spies.insert).not.toHaveBeenCalled();
    expect(fake.spies.batch).not.toHaveBeenCalled();
  });
});
