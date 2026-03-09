import { beforeEach, describe, expect, it, vi } from "vitest";

const desktopGetConnectionMock = vi.hoisted(() => vi.fn());
const desktopConnectLocalMock = vi.hoisted(() => vi.fn());
const fetchMock = vi.hoisted(() => vi.fn());
const PERSISTED_BASE_KEY = "ctxDaemonConnectionBaseV1";

const okJsonResponse = (body: unknown = { ok: true }): Response =>
  new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });

vi.mock("../utils/desktop", () => ({
  isDesktopApp: () => true,
  desktopConnectLocal: desktopConnectLocalMock,
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
    vi.stubGlobal("fetch", fetchMock);
  });

  it("bootstraps a missing local desktop connection via desktopConnectLocal", async () => {
    desktopGetConnectionMock.mockResolvedValueOnce({ kind: "none" });
    desktopConnectLocalMock.mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4399",
      token: "abc",
    });

    const mod = await import("./clientBase");
    const result = await mod.syncDesktopDaemonConnectionFromBridge({
      force: true,
      probeHealth: true,
      reason: "test_probe",
    });

    expect(desktopGetConnectionMock).toHaveBeenCalledTimes(1);
    expect(desktopConnectLocalMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).not.toHaveBeenCalled();
    expect(result.config.baseUrl).toBe("http://127.0.0.1:4399");
    expect(result.config.wsBaseUrl).toBe("ws://127.0.0.1:4399");
    expect(result.config.authToken).toBe("abc");
  });

  it("uses direct daemon fetches after one desktop bridge sync", async () => {
    desktopGetConnectionMock.mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4399",
      token: "abc",
    });
    fetchMock.mockImplementation(() => Promise.resolve(okJsonResponse({ ok: true })));

    const mod = await import("./clientBase");
    expect(mod.getDaemonClientConfig().baseUrl).toBeNull();

    const first = await mod.apiAny<{ ok: boolean }>("/api/health");
    const second = await mod.apiAny<{ ok: boolean }>("/api/health");

    expect(first.ok).toBe(true);
    expect(second.ok).toBe(true);
    expect(mod.getDaemonClientConfig().baseUrl).toBe("http://127.0.0.1:4399");
    expect(mod.getDaemonClientConfig().wsBaseUrl).toBe("ws://127.0.0.1:4399");
    expect(desktopGetConnectionMock).toHaveBeenCalledTimes(1);
    expect(desktopConnectLocalMock).not.toHaveBeenCalled();
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(fetchMock).toHaveBeenNthCalledWith(
      1,
      "http://127.0.0.1:4399/api/health",
      expect.objectContaining({
        headers: expect.objectContaining({
          authorization: "Bearer abc",
        }),
      }),
    );
  });

  it("re-syncs desktop auth when restore only has a persisted base URL", async () => {
    localStorage.setItem(PERSISTED_BASE_KEY, JSON.stringify({
      v: 1,
      baseUrl: "http://127.0.0.1:4399",
      wsBaseUrl: "ws://127.0.0.1:4399",
    }));
    desktopGetConnectionMock.mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4399",
      token: "abc",
    });
    fetchMock.mockImplementation(() => Promise.resolve(okJsonResponse({ ok: true })));

    const mod = await import("./clientBase");
    expect(mod.getDaemonClientConfig()).toMatchObject({
      baseUrl: "http://127.0.0.1:4399",
      authToken: null,
    });

    const result = await mod.apiAny<{ ok: boolean }>("/api/health");

    expect(result.ok).toBe(true);
    expect(desktopGetConnectionMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenCalledWith(
      "http://127.0.0.1:4399/api/health",
      expect.objectContaining({
        headers: expect.objectContaining({
          authorization: "Bearer abc",
        }),
      }),
    );
    expect(mod.getDaemonClientConfig()).toMatchObject({
      baseUrl: "http://127.0.0.1:4399",
      authToken: "abc",
    });
  });

  it("reads the desktop auth token after raw fetch preflight on cold start", async () => {
    desktopGetConnectionMock.mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4399",
      token: "abc",
    });
    fetchMock.mockImplementation(() => Promise.resolve(okJsonResponse({ ok: true })));

    const mod = await import("./clientBase");
    const result = await mod.daemonFetchRaw("/api/buffers/open", {
      method: "POST",
      body: JSON.stringify({ path: "README.md" }),
    });

    expect(result.status).toBe(200);
    expect(desktopGetConnectionMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenCalledWith(
      "http://127.0.0.1:4399/api/buffers/open",
      expect.objectContaining({
        headers: expect.objectContaining({
          authorization: "Bearer abc",
        }),
      }),
    );
  });

  it("preserves merged auth and caller headers for raw desktop fetches", async () => {
    desktopGetConnectionMock.mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4399",
      token: "abc",
    });
    fetchMock.mockImplementation(() => Promise.resolve(okJsonResponse({ ok: true })));

    const mod = await import("./clientBase");
    const result = await mod.daemonFetchRaw("/api/buffers/update", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "x-file-buffer-id": "buf-1",
      },
      body: JSON.stringify({ version: 2 }),
    });

    expect(result.status).toBe(200);
    expect(fetchMock).toHaveBeenCalledWith(
      "http://127.0.0.1:4399/api/buffers/update",
      expect.objectContaining({
        headers: expect.objectContaining({
          authorization: "Bearer abc",
          "content-type": "application/json",
          "x-file-buffer-id": "buf-1",
        }),
      }),
    );
  });
});
