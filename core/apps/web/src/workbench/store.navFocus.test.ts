import type { WorkbenchStore } from "./store";
import { describe, expect, it, vi } from "vitest";

vi.mock("./persistence", async () => {
  const actual = await vi.importActual<typeof import("./persistence")>("./persistence");
  return {
    ...actual,
    saveWorkbenchWindowV1: vi.fn(async () => {}),
  };
});

const getActiveTaskId = (store: WorkbenchStore): string | null => {
  const tab = store.getActiveTab();
  return tab && tab.kind === "track" ? tab.ref.taskId : null;
};

const getActiveTabKind = (store: WorkbenchStore): string | null => {
  const tab = store.getActiveTab();
  return tab ? tab.kind : null;
};

describe("WorkbenchStore navigation tokens", () => {
  it("applies system focus when the token is current", async () => {
    vi.useFakeTimers();
    try {
      const { WorkbenchStore } = await import("./store");
      const store = new WorkbenchStore("ws-1");
      const token = store.getNavToken();

      const applied = store.focusTask("task-1", "track-1", "session-1", { navToken: token, source: "system" });

      expect(applied).toBe(true);
      expect(getActiveTaskId(store)).toBe("task-1");
    } finally {
      vi.runAllTimers();
      vi.useRealTimers();
    }
  });

  it("ignores stale system focus after user navigation", async () => {
    vi.useFakeTimers();
    try {
      const { WorkbenchStore } = await import("./store");
      const store = new WorkbenchStore("ws-1");
      const token = store.getNavToken();

      store.focusTask("task-1", null, null, { source: "user" });
      const applied = store.focusTask("task-2", null, null, { navToken: token, source: "system" });

      expect(applied).toBe(false);
      expect(getActiveTaskId(store)).toBe("task-1");
    } finally {
      vi.runAllTimers();
      vi.useRealTimers();
    }
  });

  it("bumps tokens for user navigation but not system updates", async () => {
    vi.useFakeTimers();
    try {
      const { WorkbenchStore } = await import("./store");
      const store = new WorkbenchStore("ws-1");

      store.focusTask("task-1", "track-1", "session-1", { source: "user" });
      const afterUserFocus = store.getNavToken();
      store.setActiveTrackForActiveTask("track-2", { source: "user" });

      expect(store.getNavToken()).toBe(afterUserFocus + 1);
      expect(getActiveTaskId(store)).toBe("task-1");

      const token = store.getNavToken();
      store.setActiveTrackForActiveTask("track-3", { source: "system" });

      expect(store.getNavToken()).toBe(token);
      expect(getActiveTabKind(store)).toBe("track");
    } finally {
      vi.runAllTimers();
      vi.useRealTimers();
    }
  });
});
