import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  createBrowserDaemonTargetScope,
  createDesktopSshDaemonTargetScope,
  serializeDaemonTargetScope,
} from "../state/scopeIdentity";

const getDaemonConnectionMock = vi.hoisted(() => vi.fn());

vi.mock("../api/client", () => ({
  getDaemonConnection: getDaemonConnectionMock,
}));

vi.mock("../state/uiStateStore", () => ({
  uiStateBatch: vi.fn(),
  uiStateDelete: vi.fn(),
  uiStateGet: vi.fn(),
  uiStateSet: vi.fn(),
}));

describe("workbench persistence daemon scope keys", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
  });

  it("uses the serialized daemon target scope instead of the raw baseUrl", async () => {
    const targetScope = createBrowserDaemonTargetScope("https://example.com");
    getDaemonConnectionMock.mockReturnValue({
      baseUrl: "https://example.com",
      wsBaseUrl: "wss://example.com",
      authToken: null,
      runId: null,
      source: "same_origin_bootstrap",
      targetScope,
    });

    const mod = await import("./persistence");
    expect(mod.workbenchDaemonKey()).toBe(serializeDaemonTargetScope(targetScope));
    expect(mod.workbenchWindowKeyV1("ws-1", "window-1")).toContain(
      encodeURIComponent(serializeDaemonTargetScope(targetScope)),
    );
  });

  it("distinguishes reused forwarded URLs across different desktop ssh targets", async () => {
    const firstTarget = createDesktopSshDaemonTargetScope({
      host: "host-a.example",
      user: "user",
      port: 4399,
      dataDir: "/srv/ctx-a",
    });
    const secondTarget = createDesktopSshDaemonTargetScope({
      host: "host-b.example",
      user: "user",
      port: 4399,
      dataDir: "/srv/ctx-a",
    });
    getDaemonConnectionMock
      .mockReturnValueOnce({
        baseUrl: "http://127.0.0.1:4399",
        wsBaseUrl: "ws://127.0.0.1:4399",
        authToken: "abc",
        runId: null,
        source: "desktop",
        targetScope: firstTarget,
      })
      .mockReturnValueOnce({
        baseUrl: "http://127.0.0.1:4399",
        wsBaseUrl: "ws://127.0.0.1:4399",
        authToken: "abc",
        runId: null,
        source: "desktop",
        targetScope: secondTarget,
      });

    const mod = await import("./persistence");
    expect(mod.workbenchDaemonKey()).toBe(serializeDaemonTargetScope(firstTarget));
    expect(mod.workbenchDaemonKey()).toBe(serializeDaemonTargetScope(secondTarget));
    expect(serializeDaemonTargetScope(firstTarget)).not.toBe(serializeDaemonTargetScope(secondTarget));
  });
});
