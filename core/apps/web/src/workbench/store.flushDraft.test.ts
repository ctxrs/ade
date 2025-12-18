import { describe, expect, it, vi } from "vitest";

vi.mock("./persistence", async () => {
  const actual = await vi.importActual<typeof import("./persistence")>("./persistence");
  return {
    ...actual,
    saveWorkbenchDraftV1: vi.fn(async () => {}),
  };
});

describe("WorkbenchStore.flushDraft", () => {
  it("persists the current draft immediately and cancels the pending debounce", async () => {
    vi.useFakeTimers();
    try {
      const persistence = await import("./persistence");
      const saveWorkbenchDraftV1 = vi.mocked(persistence.saveWorkbenchDraftV1);
      const { WorkbenchStore } = await import("./store");
      const store = new WorkbenchStore("ws-1");
      store.setDraft("k1", { text: "hello", modeId: "default" });
      store.setDraft("k1", { text: "", modeId: "default" });

      expect(saveWorkbenchDraftV1).toHaveBeenCalledTimes(0);

      await store.flushDraft("k1");

      expect(saveWorkbenchDraftV1).toHaveBeenCalledTimes(1);
      const [_workspaceId, _key, draft] = saveWorkbenchDraftV1.mock.calls[0] ?? [];
      expect(draft).toEqual(
        expect.objectContaining({
          text: "",
          modeId: "default",
          updatedAtMs: expect.any(Number),
        }),
      );

      vi.runAllTimers();
      expect(saveWorkbenchDraftV1).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });
});
