import { beforeEach, describe, expect, it, vi } from "vitest";

const desktopGetConnectionMock = vi.hoisted(() => vi.fn());
const desktopConnectLocalMock = vi.hoisted(() => vi.fn());
const fetchMock = vi.hoisted(() => vi.fn());
const SESSION_CONNECTION_KEY = "ctxDaemonConnectionV1";
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
    desktopGetConnectionMock.mockResolvedValueOnce({
      kind: "none",
      intent: "auto_local_bootstrap",
      local_auto_bootstrap_allowed: true,
    });
    desktopConnectLocalMock.mockResolvedValue({
      kind: "local",
      intent: "explicit_local",
      local_auto_bootstrap_allowed: true,
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

  it("does not auto-connect local after an explicit desktop disconnect", async () => {
    desktopGetConnectionMock.mockResolvedValueOnce({
      kind: "none",
      intent: "explicit_disconnected",
      local_auto_bootstrap_allowed: false,
    });

    const mod = await import("./clientBase");
    const result = await mod.syncDesktopDaemonConnectionFromBridge({
      force: true,
      probeHealth: true,
      reason: "test_explicit_disconnect",
    });

    expect(desktopGetConnectionMock).toHaveBeenCalledTimes(1);
    expect(desktopConnectLocalMock).not.toHaveBeenCalled();
    expect(result.config.baseUrl).toBeNull();
    expect(result.config.authToken).toBeNull();
  });

  it("republishes canonical desktop state when a bridge read clears a stale connection", async () => {
    desktopGetConnectionMock.mockResolvedValue({
      kind: "none",
      intent: "explicit_disconnected",
      local_auto_bootstrap_allowed: false,
      base_url: null,
      token: null,
    });

    const mod = await import("./clientBase");
    mod.applyDaemonDesktopConnection({
      base_url: "http://127.0.0.1:4399",
      token: "stale-token",
    });
    expect(mod.getDaemonClientConfig()).toMatchObject({
      baseUrl: "http://127.0.0.1:4399",
      authToken: "stale-token",
    });

    const result = await mod.syncDesktopDaemonConnectionFromBridge({
      force: true,
      probeHealth: false,
      reason: "test_clear_stale_connection",
    });

    expect(desktopGetConnectionMock).toHaveBeenCalledTimes(1);
    expect(desktopConnectLocalMock).not.toHaveBeenCalled();
    expect(result.config).toMatchObject({
      baseUrl: null,
      wsBaseUrl: null,
      authToken: null,
    });
    expect(mod.getDaemonClientConfig()).toMatchObject({
      baseUrl: null,
      wsBaseUrl: null,
      authToken: null,
    });
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

  it("refreshes desktop bridge state before reuse after a later token rotation", async () => {
    const dateNowSpy = vi.spyOn(Date, "now");
    let now = 1_000;
    dateNowSpy.mockImplementation(() => now);
    desktopGetConnectionMock
      .mockResolvedValueOnce({
        kind: "local",
        base_url: "http://127.0.0.1:4399",
        token: "token-old",
      })
      .mockResolvedValueOnce({
        kind: "local",
        base_url: "http://127.0.0.1:4400",
        token: "token-new",
      });
    fetchMock.mockImplementation(() => Promise.resolve(okJsonResponse({ ok: true })));

    try {
      const mod = await import("./clientBase");
      await mod.apiAny<{ ok: boolean }>("/api/health");

      now = 3_000;
      await mod.apiAny<{ ok: boolean }>("/api/providers");

      expect(desktopGetConnectionMock).toHaveBeenCalledTimes(2);
      expect(fetchMock).toHaveBeenNthCalledWith(
        1,
        "http://127.0.0.1:4399/api/health",
        expect.objectContaining({
          headers: expect.objectContaining({
            authorization: "Bearer token-old",
          }),
        }),
      );
      expect(fetchMock).toHaveBeenNthCalledWith(
        2,
        "http://127.0.0.1:4400/api/providers",
        expect.objectContaining({
          headers: expect.objectContaining({
            authorization: "Bearer token-new",
          }),
        }),
      );
      expect(mod.getDaemonClientConfig()).toMatchObject({
        baseUrl: "http://127.0.0.1:4400",
        wsBaseUrl: "ws://127.0.0.1:4400",
        authToken: "token-new",
      });
    } finally {
      dateNowSpy.mockRestore();
    }
  });

  it("refreshes daemon target scope when the desktop bridge reuses the same forwarded base URL", async () => {
    const dateNowSpy = vi.spyOn(Date, "now");
    let now = 1_000;
    dateNowSpy.mockImplementation(() => now);
    desktopGetConnectionMock
      .mockResolvedValueOnce({
        kind: "ssh",
        base_url: "http://127.0.0.1:4399",
        token: "abc",
        host: "host-a.example",
        user: "user",
        remote_port: 4399,
        remote_data_dir: "/srv/ctx-a",
      })
      .mockResolvedValueOnce({
        kind: "ssh",
        base_url: "http://127.0.0.1:4399",
        token: "abc",
        host: "host-b.example",
        user: "user",
        remote_port: 4399,
        remote_data_dir: "/srv/ctx-a",
      });
    fetchMock.mockImplementation(() => Promise.resolve(okJsonResponse({ ok: true })));

    try {
      const mod = await import("./clientBase");
      const daemonConnection = await import("./daemonConnection");
      await mod.apiAny<{ ok: boolean }>("/api/health");
      expect(daemonConnection.getDaemonConnection().targetScope).toMatchObject({
        kind: "desktop_ssh",
        host: "host-a.example",
        user: "user",
        port: 4399,
        dataDir: "/srv/ctx-a",
      });

      now = 3_000;
      await mod.apiAny<{ ok: boolean }>("/api/providers");

      expect(desktopGetConnectionMock).toHaveBeenCalledTimes(2);
      expect(fetchMock).toHaveBeenNthCalledWith(
        1,
        "http://127.0.0.1:4399/api/health",
        expect.objectContaining({
          headers: expect.objectContaining({
            authorization: "Bearer abc",
          }),
        }),
      );
      expect(fetchMock).toHaveBeenNthCalledWith(
        2,
        "http://127.0.0.1:4399/api/providers",
        expect.objectContaining({
          headers: expect.objectContaining({
            authorization: "Bearer abc",
          }),
        }),
      );
      expect(daemonConnection.getDaemonConnection().targetScope).toMatchObject({
        kind: "desktop_ssh",
        host: "host-b.example",
        user: "user",
        port: 4399,
        dataDir: "/srv/ctx-a",
      });
      expect(String(JSON.parse(sessionStorage.getItem(SESSION_CONNECTION_KEY) ?? "{}").targetScope)).toContain("host-b.example");
      expect(String(JSON.parse(localStorage.getItem(PERSISTED_BASE_KEY) ?? "{}").targetScope)).toContain("host-b.example");
    } finally {
      dateNowSpy.mockRestore();
    }
  });

  it("repopulates canonical session and persisted base storage after a later bridge rotation", async () => {
    sessionStorage.setItem(
      SESSION_CONNECTION_KEY,
      JSON.stringify({
        v: 1,
        baseUrl: "http://127.0.0.1:4399",
        wsBaseUrl: "ws://127.0.0.1:4399",
        authToken: "token-old",
        source: "desktop",
      }),
    );
    localStorage.setItem(
      PERSISTED_BASE_KEY,
      JSON.stringify({
        v: 1,
        baseUrl: "http://127.0.0.1:4399",
        wsBaseUrl: "ws://127.0.0.1:4399",
      }),
    );
    desktopGetConnectionMock.mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4400",
      token: "token-new",
    });

    const mod = await import("./clientBase");
    expect(mod.getDaemonClientConfig()).toMatchObject({
      baseUrl: "http://127.0.0.1:4399",
      wsBaseUrl: "ws://127.0.0.1:4399",
      authToken: null,
    });

    const result = await mod.syncDesktopDaemonConnectionFromBridge({
      force: true,
      reason: "test_rotation",
    });

    expect(desktopGetConnectionMock).toHaveBeenCalledTimes(1);
    expect(result.config).toMatchObject({
      baseUrl: "http://127.0.0.1:4400",
      wsBaseUrl: "ws://127.0.0.1:4400",
      authToken: "token-new",
    });
    expect(JSON.parse(sessionStorage.getItem(SESSION_CONNECTION_KEY) ?? "{}")).toMatchObject({
      v: 1,
      baseUrl: "http://127.0.0.1:4400",
      wsBaseUrl: "ws://127.0.0.1:4400",
      authToken: null,
      source: "desktop",
    });
    expect(JSON.parse(localStorage.getItem(PERSISTED_BASE_KEY) ?? "{}")).toMatchObject({
      v: 1,
      baseUrl: "http://127.0.0.1:4400",
      wsBaseUrl: "ws://127.0.0.1:4400",
    });
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
    expect(fetchMock).toHaveBeenNthCalledWith(
      1,
      "http://127.0.0.1:4399/api/workspaces",
      expect.objectContaining({
        headers: expect.objectContaining({
          authorization: "Bearer abc",
        }),
        signal: expect.any(AbortSignal),
      }),
    );
    expect(fetchMock).toHaveBeenNthCalledWith(
      2,
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

  it("repairs stale restored desktop auth before the first API request", async () => {
    localStorage.setItem(PERSISTED_BASE_KEY, JSON.stringify({
      v: 1,
      baseUrl: "http://127.0.0.1:4399",
      wsBaseUrl: "ws://127.0.0.1:4399",
    }));
    desktopGetConnectionMock.mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4399",
      token: "stale-token",
    });
    desktopConnectLocalMock.mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4400",
      token: "fresh-token",
    });
    fetchMock
      .mockResolvedValueOnce(new Response(JSON.stringify({ error: "unauthorized" }), { status: 401 }))
      .mockResolvedValueOnce(okJsonResponse({ ok: true }));

    const mod = await import("./clientBase");
    const result = await mod.apiAny<{ ok: boolean }>("/api/health");

    expect(result.ok).toBe(true);
    expect(desktopGetConnectionMock).toHaveBeenCalledTimes(1);
    expect(desktopConnectLocalMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenNthCalledWith(
      1,
      "http://127.0.0.1:4399/api/workspaces",
      expect.objectContaining({
        headers: expect.objectContaining({
          authorization: "Bearer stale-token",
        }),
        signal: expect.any(AbortSignal),
      }),
    );
    expect(fetchMock).toHaveBeenNthCalledWith(
      2,
      "http://127.0.0.1:4400/api/health",
      expect.objectContaining({
        headers: expect.objectContaining({
          authorization: "Bearer fresh-token",
        }),
      }),
    );
    expect(mod.getDaemonClientConfig()).toMatchObject({
      baseUrl: "http://127.0.0.1:4400",
      authToken: "fresh-token",
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
    const result = await mod.daemonFetchRaw("/api/sessions/web", {
      method: "POST",
      body: JSON.stringify({ label: "preflight-check" }),
    });

    expect(result.status).toBe(200);
    expect(desktopGetConnectionMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenCalledWith(
      "http://127.0.0.1:4399/api/sessions/web",
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
    const result = await mod.daemonFetchRaw("/api/sessions/web", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "x-test-case": "raw-fetch-merge",
      },
      body: JSON.stringify({ label: "header-merge" }),
    });

    expect(result.status).toBe(200);
    expect(fetchMock).toHaveBeenCalledWith(
      "http://127.0.0.1:4399/api/sessions/web",
      expect.objectContaining({
        headers: expect.objectContaining({
          authorization: "Bearer abc",
          "content-type": "application/json",
          "x-test-case": "raw-fetch-merge",
        }),
      }),
    );
  });
});
