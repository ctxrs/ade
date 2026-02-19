import { beforeEach, describe, expect, it, vi } from "vitest";

const desktopDaemonRequestMock = vi.hoisted(() => vi.fn());
const desktopGetConnectionMock = vi.hoisted(() => vi.fn());

vi.mock("../utils/desktop", () => ({
  isDesktopApp: () => true,
  desktopDaemonRequest: desktopDaemonRequestMock,
  desktopGetConnection: desktopGetConnectionMock,
}));

describe("clientBase desktop connection sync", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
    sessionStorage.clear();
    localStorage.clear();
    const g = globalThis as typeof globalThis & { __TAURI__?: unknown; __TAURI_INTERNALS__?: unknown };
    g.__TAURI__ = {};
  });

  it("probes desktop daemon health when bridge connection is initially missing", async () => {
    desktopGetConnectionMock
      .mockResolvedValueOnce({ kind: "none" })
      .mockResolvedValueOnce({ kind: "local", base_url: "http://127.0.0.1:4399", token: "abc" });
    desktopDaemonRequestMock.mockResolvedValue({
      status: 200,
      body: JSON.stringify({ ok: true }),
      content_type: "application/json",
    });

    const mod = await import("./clientBase");
    const result = await mod.syncDesktopDaemonConnectionFromBridge({
      force: true,
      probeHealth: true,
      reason: "test_probe",
    });

    expect(desktopDaemonRequestMock).toHaveBeenCalledWith({
      method: "GET",
      path: "/api/health",
      body: null,
      headers: [["content-type", "application/json"]],
    });
    expect(result.config.baseUrl).toBe("http://127.0.0.1:4399");
    expect(result.config.wsBaseUrl).toBe("ws://127.0.0.1:4399");
    expect(result.config.authToken).toBe("abc");
  });

  it("hydrates daemon base via desktop bridge before desktop apiAny request", async () => {
    desktopGetConnectionMock.mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4399",
      token: "abc",
    });
    desktopDaemonRequestMock.mockResolvedValue({
      status: 200,
      body: JSON.stringify({ ok: true }),
      content_type: "application/json",
    });

    const mod = await import("./clientBase");
    expect(mod.getDaemonClientConfig().baseUrl).toBeNull();

    const resp = await mod.apiAny<{ ok: boolean }>("/api/health");
    expect(resp.ok).toBe(true);
    expect(mod.getDaemonClientConfig().baseUrl).toBe("http://127.0.0.1:4399");
    expect(mod.getDaemonClientConfig().wsBaseUrl).toBe("ws://127.0.0.1:4399");
    expect(desktopDaemonRequestMock).toHaveBeenCalledTimes(1);
  });
});
