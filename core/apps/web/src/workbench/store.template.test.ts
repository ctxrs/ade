import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PersistedWorkbenchTemplateV1 } from "./types";

const loadWorkbenchTemplateV1Mock = vi.hoisted(() => vi.fn(async () => null as PersistedWorkbenchTemplateV1 | null));
const saveWorkbenchTemplateV1ImmediateMock = vi.hoisted(() => vi.fn(async () => {}));

vi.mock("./persistence", async () => {
  const actual = await vi.importActual<typeof import("./persistence")>("./persistence");
  return {
    ...actual,
    loadWorkbenchDraftV1: vi.fn(async () => null),
    loadWorkbenchWindowV1: vi.fn(async () => null),
    loadWorkbenchTemplateV1: loadWorkbenchTemplateV1Mock,
    saveWorkbenchDraftV1: vi.fn(async () => {}),
    saveWorkbenchWindowV1Immediate: vi.fn(async () => {}),
    saveWorkbenchTemplateV1Immediate: saveWorkbenchTemplateV1ImmediateMock,
  };
});

describe("WorkbenchStore template state", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sessionStorage.clear();
    window.name = "";
  });

  it("defaults to classic and persists template switches per window", async () => {
    const { WorkbenchStore } = await import("./store");
    const store = new WorkbenchStore("ws-1");

    expect(store.getTemplateState()).toEqual({ id: "classic", version: 1, layout: {} });
    expect(store.setTemplateId("multipane")).toBe(true);
    expect(store.getTemplateState()).toEqual({ id: "multipane", version: 1, layout: {} });

    const { workspaceId, windowId } = store.getSnapshot();
    expect(saveWorkbenchTemplateV1ImmediateMock).toHaveBeenCalledWith(workspaceId, windowId, {
      v: 1,
      template: { id: "multipane", version: 1, layout: {} },
    });
  });

  it("hydrates persisted template state when no session override exists", async () => {
    loadWorkbenchTemplateV1Mock.mockResolvedValueOnce({
      v: 1,
      template: { id: "review", version: 1, layout: {} },
    });
    const { WorkbenchStore } = await import("./store");
    const store = new WorkbenchStore("ws-1");

    store.init();

    await vi.waitFor(() => expect(store.getSnapshot().hydrated).toBe(true));
    expect(store.getTemplateState()).toEqual({ id: "review", version: 1, layout: {} });
  });

  it("persists split pane focus and resize actions through the workbench window", async () => {
    const { WorkbenchStore } = await import("./store");
    const store = new WorkbenchStore("ws-1");
    const firstLeaf = store.getSnapshot().window.focusedLeafId;
    const initialNavToken = store.getNavToken();

    expect(store.splitFocusedLeaf("horizontal")).toBe(true);
    expect(store.getNavToken()).toBeGreaterThan(initialNavToken);
    const splitWindow = store.getSnapshot().window;
    expect(splitWindow.layout.kind).toBe("split");
    if (splitWindow.layout.kind !== "split") throw new Error("expected split layout");
    expect(splitWindow.layout.direction).toBe("horizontal");
    expect(splitWindow.focusedLeafId).not.toBe(firstLeaf);

    const splitNavToken = store.getNavToken();
    expect(store.focusLeaf(firstLeaf)).toBe(true);
    expect(store.getNavToken()).toBeGreaterThan(splitNavToken);
    expect(store.getSnapshot().window.focusedLeafId).toBe(firstLeaf);

    const focusNavToken = store.getNavToken();
    expect(store.resizeSplit(splitWindow.layout.id, 0.8)).toBe(true);
    expect(store.getNavToken()).toBe(focusNavToken);
    const resized = store.getSnapshot().window.layout;
    expect(resized.kind).toBe("split");
    if (resized.kind !== "split") throw new Error("expected split layout");
    expect(resized.ratio).toBe(0.8);
    expect(store.resizeSplit("missing", 0.2)).toBe(false);
  });
});
