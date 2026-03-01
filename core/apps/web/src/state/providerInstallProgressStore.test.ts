import { beforeEach, describe, expect, it } from "vitest";
import {
  clearProviderInstallProgress,
  getProviderInstallProgressSnapshot,
  subscribeProviderInstallProgress,
  upsertProviderInstallProgress,
} from "./providerInstallProgressStore";

describe("providerInstallProgressStore", () => {
  beforeEach(() => {
    clearProviderInstallProgress();
  });

  it("does not emit for identical upserts", () => {
    const snapshots: ReturnType<typeof getProviderInstallProgressSnapshot>[] = [];
    const unsubscribe = subscribeProviderInstallProgress((snapshot) => {
      snapshots.push(snapshot);
    });

    upsertProviderInstallProgress("amp", {
      installId: "install-1",
      state: "running",
      pct: 40,
      target: "host",
      errorCode: undefined,
      error: undefined,
    });
    upsertProviderInstallProgress("amp", {
      installId: "install-1",
      state: "running",
      pct: 40,
      target: "host",
      errorCode: undefined,
      error: undefined,
    });

    unsubscribe();

    expect(snapshots).toHaveLength(2);
    expect(snapshots[1].amp?.state).toBe("running");
    expect(snapshots[1].amp?.pct).toBe(40);
  });

  it("emits when install state changes", () => {
    const snapshots: ReturnType<typeof getProviderInstallProgressSnapshot>[] = [];
    const unsubscribe = subscribeProviderInstallProgress((snapshot) => {
      snapshots.push(snapshot);
    });

    upsertProviderInstallProgress("goose", {
      installId: "install-2",
      state: "running",
      pct: 10,
      target: "host",
      errorCode: undefined,
      error: undefined,
    });
    upsertProviderInstallProgress("goose", {
      installId: "install-2",
      state: "succeeded",
      pct: 100,
      target: "host",
      errorCode: undefined,
      error: undefined,
    });

    unsubscribe();

    expect(snapshots).toHaveLength(3);
    expect(snapshots[2].goose?.state).toBe("succeeded");
    expect(snapshots[2].goose?.pct).toBe(100);
  });
});
