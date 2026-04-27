import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const desktopGetConnectionMock = vi.hoisted(() => vi.fn());
const desktopConnectLocalMock = vi.hoisted(() => vi.fn());
const desktopDaemonRequestMock = vi.hoisted(() => vi.fn());
const fetchMock = vi.hoisted(() => vi.fn());
const SESSION_CONNECTION_KEY = "ctxDaemonConnectionV1";
const PERSISTED_BASE_KEY = "ctxDaemonConnectionBaseV1";

const okDesktopJsonResponse = (body: unknown = { ok: true }) => ({
  status: 200,
  body: JSON.stringify(body),
  content_type: "application/json",
});

const errorDesktopJsonResponse = (status: number, body: unknown) => ({
  status,
  body: JSON.stringify(body),
  content_type: "application/json",
});

const expectNoAuthorizationHeader = (headers: Array<[string, string]>) => {
  expect(headers.some(([key]) => key.toLowerCase() === "authorization")).toBe(false);
};

vi.mock("../utils/desktop", () => ({
  isDesktopApp: () => true,
  desktopConnectLocal: desktopConnectLocalMock,
  desktopDaemonRequest: desktopDaemonRequestMock,
  desktopGetConnection: desktopGetConnectionMock,
}));

let clientBaseMod: typeof import("./clientBase");
let daemonConnectionMod: typeof import("./daemonConnection");
let desktopDaemonConnectionMod: typeof import("./desktopDaemonConnection");
let clientBaseTelemetryMod: typeof import("./clientBaseTelemetry");

describe("clientBase desktop connection sync", () => {
  beforeAll(async () => {
    clientBaseMod = await import("./clientBase");
    daemonConnectionMod = await import("./daemonConnection");
    desktopDaemonConnectionMod = await import("./desktopDaemonConnection");
    clientBaseTelemetryMod = await import("./clientBaseTelemetry");
  }, 60_000);

  beforeEach(() => {
    vi.clearAllMocks();
    sessionStorage.clear();
    localStorage.clear();
    const g = globalThis as typeof globalThis & { __TAURI__?: unknown; __TAURI_INTERNALS__?: unknown };
    g.__TAURI__ = {};
    vi.stubGlobal("fetch", fetchMock);
    clientBaseTelemetryMod.resetClientBaseTelemetryForTests();
    desktopDaemonConnectionMod.resetDesktopDaemonConnectionSyncForTests();
    daemonConnectionMod.resetDaemonConnectionStateForTests();
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
      browser_query_secret: "browser-secret-abc",
    });

    const result = await clientBaseMod.syncDesktopDaemonConnectionFromBridge({
      force: true,
      probeHealth: true,
      reason: "test_probe",
    });

    expect(desktopGetConnectionMock).toHaveBeenCalledTimes(1);
    expect(desktopConnectLocalMock).toHaveBeenCalledTimes(1);
    expect(desktopDaemonRequestMock).not.toHaveBeenCalled();
    expect(fetchMock).not.toHaveBeenCalled();
    expect(result.config.baseUrl).toBe("http://127.0.0.1:4399");
    expect(result.config.wsBaseUrl).toBe("ws://127.0.0.1:4399");
    expect(result.config.authToken).toBe("browser-secret-abc");
  });

  it("does not auto-connect local after an explicit desktop disconnect", async () => {
    desktopGetConnectionMock.mockResolvedValueOnce({
      kind: "none",
      intent: "explicit_disconnected",
      local_auto_bootstrap_allowed: false,
    });

    const result = await clientBaseMod.syncDesktopDaemonConnectionFromBridge({
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
      browser_query_secret: null,
    });

    clientBaseMod.applyDaemonDesktopConnection({
      base_url: "http://127.0.0.1:4399",
      browser_query_secret: "stale-browser-secret",
    });
    expect(clientBaseMod.getDaemonClientConfig()).toMatchObject({
      baseUrl: "http://127.0.0.1:4399",
      authToken: "stale-browser-secret",
    });

    const result = await clientBaseMod.syncDesktopDaemonConnectionFromBridge({
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
    expect(clientBaseMod.getDaemonClientConfig()).toMatchObject({
      baseUrl: null,
      wsBaseUrl: null,
      authToken: null,
    });
  });

  it("uses the desktop daemon proxy after one desktop bridge sync", async () => {
    desktopGetConnectionMock.mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4399",
      browser_query_secret: "browser-secret-abc",
    });
    desktopDaemonRequestMock.mockResolvedValue(okDesktopJsonResponse({ ok: true }));

    expect(clientBaseMod.getDaemonClientConfig().baseUrl).toBeNull();

    const first = await clientBaseMod.apiAny<{ ok: boolean }>("/api/health");
    const second = await clientBaseMod.apiAny<{ ok: boolean }>("/api/health");

    expect(first.ok).toBe(true);
    expect(second.ok).toBe(true);
    expect(clientBaseMod.getDaemonClientConfig().baseUrl).toBe("http://127.0.0.1:4399");
    expect(clientBaseMod.getDaemonClientConfig().wsBaseUrl).toBe("ws://127.0.0.1:4399");
    expect(desktopGetConnectionMock).toHaveBeenCalledTimes(1);
    expect(desktopConnectLocalMock).not.toHaveBeenCalled();
    expect(fetchMock).not.toHaveBeenCalled();
    expect(desktopDaemonRequestMock).toHaveBeenCalledTimes(2);
    expect(desktopDaemonRequestMock).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({
        method: "GET",
        path: "/api/health",
        body: null,
        headers: expect.arrayContaining([["content-type", "application/json"]]),
      }),
    );
    expectNoAuthorizationHeader(desktopDaemonRequestMock.mock.calls[0]?.[0]?.headers ?? []);
  });

  it("refreshes desktop bridge state before reuse after a later token rotation", async () => {
    const dateNowSpy = vi.spyOn(Date, "now");
    let now = 1_000;
    dateNowSpy.mockImplementation(() => now);
    desktopGetConnectionMock
      .mockResolvedValueOnce({
        kind: "local",
        base_url: "http://127.0.0.1:4399",
        browser_query_secret: "browser-secret-old",
      })
      .mockResolvedValueOnce({
        kind: "local",
        base_url: "http://127.0.0.1:4400",
        browser_query_secret: "browser-secret-new",
      });
    desktopDaemonRequestMock.mockResolvedValue(okDesktopJsonResponse({ ok: true }));

    try {
      await clientBaseMod.apiAny<{ ok: boolean }>("/api/health");

      now = 3_000;
      await clientBaseMod.apiAny<{ ok: boolean }>("/api/providers");

      expect(desktopGetConnectionMock).toHaveBeenCalledTimes(2);
      expect(fetchMock).not.toHaveBeenCalled();
      expect(desktopDaemonRequestMock).toHaveBeenNthCalledWith(
        1,
        expect.objectContaining({
          method: "GET",
          path: "/api/health",
        }),
      );
      expect(desktopDaemonRequestMock).toHaveBeenNthCalledWith(
        2,
        expect.objectContaining({
          method: "GET",
          path: "/api/providers",
        }),
      );
      expectNoAuthorizationHeader(desktopDaemonRequestMock.mock.calls[0]?.[0]?.headers ?? []);
      expectNoAuthorizationHeader(desktopDaemonRequestMock.mock.calls[1]?.[0]?.headers ?? []);
      expect(clientBaseMod.getDaemonClientConfig()).toMatchObject({
        baseUrl: "http://127.0.0.1:4400",
        wsBaseUrl: "ws://127.0.0.1:4400",
        authToken: "browser-secret-new",
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
        browser_query_secret: "browser-secret-ssh",
        host: "host-a.example",
        user: "user",
        remote_port: 4399,
        remote_data_dir: "/srv/ctx-a",
      })
      .mockResolvedValueOnce({
        kind: "ssh",
        base_url: "http://127.0.0.1:4399",
        browser_query_secret: "browser-secret-ssh",
        host: "host-b.example",
        user: "user",
        remote_port: 4399,
        remote_data_dir: "/srv/ctx-a",
      });
    desktopDaemonRequestMock.mockResolvedValue(okDesktopJsonResponse({ ok: true }));

    try {
      await clientBaseMod.apiAny<{ ok: boolean }>("/api/health");
      expect(daemonConnectionMod.getDaemonConnection().targetScope).toMatchObject({
        kind: "desktop_ssh",
        host: "host-a.example",
        user: "user",
        port: 4399,
        dataDir: "/srv/ctx-a",
      });

      now = 3_000;
      await clientBaseMod.apiAny<{ ok: boolean }>("/api/providers");

      expect(desktopGetConnectionMock).toHaveBeenCalledTimes(2);
      expect(fetchMock).not.toHaveBeenCalled();
      expect(desktopDaemonRequestMock).toHaveBeenNthCalledWith(
        1,
        expect.objectContaining({
          method: "GET",
          path: "/api/health",
        }),
      );
      expect(desktopDaemonRequestMock).toHaveBeenNthCalledWith(
        2,
        expect.objectContaining({
          method: "GET",
          path: "/api/providers",
        }),
      );
      expectNoAuthorizationHeader(desktopDaemonRequestMock.mock.calls[0]?.[0]?.headers ?? []);
      expectNoAuthorizationHeader(desktopDaemonRequestMock.mock.calls[1]?.[0]?.headers ?? []);
      expect(daemonConnectionMod.getDaemonConnection().targetScope).toMatchObject({
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
    daemonConnectionMod.resetDaemonConnectionStateForTests();
    desktopGetConnectionMock.mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4400",
      browser_query_secret: "browser-secret-new",
    });

    expect(clientBaseMod.getDaemonClientConfig()).toMatchObject({
      baseUrl: "http://127.0.0.1:4399",
      wsBaseUrl: "ws://127.0.0.1:4399",
      authToken: null,
    });

    const result = await clientBaseMod.syncDesktopDaemonConnectionFromBridge({
      force: true,
      reason: "test_rotation",
    });

    expect(desktopGetConnectionMock).toHaveBeenCalledTimes(1);
    expect(result.config).toMatchObject({
      baseUrl: "http://127.0.0.1:4400",
      wsBaseUrl: "ws://127.0.0.1:4400",
      authToken: "browser-secret-new",
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
    daemonConnectionMod.resetDaemonConnectionStateForTests();
    desktopGetConnectionMock.mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4399",
      browser_query_secret: "browser-secret-abc",
    });
    desktopDaemonRequestMock
      .mockResolvedValueOnce(okDesktopJsonResponse([]))
      .mockResolvedValueOnce(okDesktopJsonResponse({ ok: true }));

    expect(clientBaseMod.getDaemonClientConfig()).toMatchObject({
      baseUrl: "http://127.0.0.1:4399",
      authToken: null,
    });

    const result = await clientBaseMod.apiAny<{ ok: boolean }>("/api/health");

    expect(result.ok).toBe(true);
    expect(desktopGetConnectionMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).not.toHaveBeenCalled();
    expect(desktopDaemonRequestMock).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({
        method: "GET",
        path: "/api/workspaces",
      }),
    );
    expect(desktopDaemonRequestMock).toHaveBeenNthCalledWith(
      2,
      expect.objectContaining({
        method: "GET",
        path: "/api/health",
      }),
    );
    expectNoAuthorizationHeader(desktopDaemonRequestMock.mock.calls[0]?.[0]?.headers ?? []);
    expectNoAuthorizationHeader(desktopDaemonRequestMock.mock.calls[1]?.[0]?.headers ?? []);
    expect(clientBaseMod.getDaemonClientConfig()).toMatchObject({
      baseUrl: "http://127.0.0.1:4399",
      authToken: "browser-secret-abc",
    });
  });

  it("repairs stale restored desktop auth before the first API request", async () => {
    localStorage.setItem(PERSISTED_BASE_KEY, JSON.stringify({
      v: 1,
      baseUrl: "http://127.0.0.1:4399",
      wsBaseUrl: "ws://127.0.0.1:4399",
    }));
    daemonConnectionMod.resetDaemonConnectionStateForTests();
    desktopGetConnectionMock.mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4399",
      browser_query_secret: "browser-secret-stale",
    });
    desktopConnectLocalMock.mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4400",
      browser_query_secret: "browser-secret-fresh",
    });
    desktopDaemonRequestMock
      .mockResolvedValueOnce(errorDesktopJsonResponse(401, { error: "unauthorized" }))
      .mockResolvedValueOnce(okDesktopJsonResponse({ ok: true }));

    const result = await clientBaseMod.apiAny<{ ok: boolean }>("/api/health");

    expect(result.ok).toBe(true);
    expect(desktopGetConnectionMock).toHaveBeenCalledTimes(1);
    expect(desktopConnectLocalMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).not.toHaveBeenCalled();
    expect(desktopDaemonRequestMock).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({
        method: "GET",
        path: "/api/workspaces",
      }),
    );
    expect(desktopDaemonRequestMock).toHaveBeenNthCalledWith(
      2,
      expect.objectContaining({
        method: "GET",
        path: "/api/health",
      }),
    );
    expectNoAuthorizationHeader(desktopDaemonRequestMock.mock.calls[0]?.[0]?.headers ?? []);
    expectNoAuthorizationHeader(desktopDaemonRequestMock.mock.calls[1]?.[0]?.headers ?? []);
    expect(clientBaseMod.getDaemonClientConfig()).toMatchObject({
      baseUrl: "http://127.0.0.1:4400",
      authToken: "browser-secret-fresh",
    });
  });

  it("uses the desktop daemon proxy after raw fetch preflight on cold start", async () => {
    desktopGetConnectionMock.mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4399",
      browser_query_secret: "browser-secret-abc",
    });
    desktopDaemonRequestMock.mockResolvedValue(okDesktopJsonResponse({ ok: true }));

    const result = await clientBaseMod.daemonFetchRaw("/api/sessions/web", {
      method: "POST",
      body: JSON.stringify({ label: "preflight-check" }),
    });

    expect(result.status).toBe(200);
    expect(desktopGetConnectionMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).not.toHaveBeenCalled();
    expect(desktopDaemonRequestMock).toHaveBeenCalledWith(
      expect.objectContaining({
        method: "POST",
        path: "/api/sessions/web",
        body: JSON.stringify({ label: "preflight-check" }),
      }),
    );
    expectNoAuthorizationHeader(desktopDaemonRequestMock.mock.calls[0]?.[0]?.headers ?? []);
  });

  it("preserves caller headers for raw desktop proxy requests", async () => {
    desktopGetConnectionMock.mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4399",
      browser_query_secret: "browser-secret-abc",
    });
    desktopDaemonRequestMock.mockResolvedValue(okDesktopJsonResponse({ ok: true }));

    const result = await clientBaseMod.daemonFetchRaw("/api/sessions/web", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "x-test-case": "raw-fetch-merge",
      },
      body: JSON.stringify({ label: "header-merge" }),
    });

    expect(result.status).toBe(200);
    expect(fetchMock).not.toHaveBeenCalled();
    expect(desktopDaemonRequestMock).toHaveBeenCalledWith(
      expect.objectContaining({
        method: "POST",
        path: "/api/sessions/web",
        body: JSON.stringify({ label: "header-merge" }),
        headers: expect.arrayContaining([
          ["content-type", "application/json"],
          ["x-test-case", "raw-fetch-merge"],
        ]),
      }),
    );
    expectNoAuthorizationHeader(desktopDaemonRequestMock.mock.calls[0]?.[0]?.headers ?? []);
  });

  it("strips caller Authorization headers before desktop proxying", async () => {
    desktopGetConnectionMock.mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4399",
      browser_query_secret: "browser-secret-abc",
    });
    desktopDaemonRequestMock.mockResolvedValue(okDesktopJsonResponse({ ok: true }));

    const result = await clientBaseMod.daemonFetchRaw("/api/sessions/web", {
      method: "POST",
      headers: {
        Authorization: "Bearer should-not-pass-through",
        "content-type": "application/json",
        "x-test-case": "strip-authorization",
      },
      body: JSON.stringify({ label: "strip-authorization" }),
    });

    expect(result.status).toBe(200);
    expect(fetchMock).not.toHaveBeenCalled();
    expect(desktopDaemonRequestMock).toHaveBeenCalledWith(
      expect.objectContaining({
        method: "POST",
        path: "/api/sessions/web",
        body: JSON.stringify({ label: "strip-authorization" }),
        headers: expect.arrayContaining([
          ["content-type", "application/json"],
          ["x-test-case", "strip-authorization"],
        ]),
      }),
    );
    expectNoAuthorizationHeader(desktopDaemonRequestMock.mock.calls[0]?.[0]?.headers ?? []);
  });
});
