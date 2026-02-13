import { beforeEach, describe, expect, it, vi } from "vitest";

const SESSION_CONNECTION_KEY = "ctxDaemonConnectionV1";
const LOCAL_PERSISTED_BASE_KEY = "ctxDaemonConnectionBaseV1";

describe("daemonConnection", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.unstubAllEnvs();
    sessionStorage.clear();
    localStorage.clear();
    window.history.replaceState({}, "", "/");
    const g = globalThis as typeof globalThis & { __TAURI_INTERNALS__?: unknown; __TAURI__?: unknown };
    delete g.__TAURI_INTERNALS__;
    delete g.__TAURI__;
  });

  it("normalizes base/ws urls consistently", async () => {
    const mod = await import("./daemonConnection");

    expect(mod.normalizeDaemonBaseUrl("https://example.com///")).toBe("https://example.com");
    expect(mod.normalizeDaemonBaseUrl("wss://example.com///")).toBe("https://example.com");
    expect(mod.normalizeDaemonWsBaseUrl("https://example.com///")).toBe("wss://example.com");
    expect(mod.normalizeDaemonWsBaseUrl("ws://example.com///")).toBe("ws://example.com");
    expect(mod.deriveDaemonWsBaseUrl("http://127.0.0.1:4399")).toBe("ws://127.0.0.1:4399");
  });

  it("restores canonical base from persisted canonical storage", async () => {
    localStorage.setItem(
      LOCAL_PERSISTED_BASE_KEY,
      JSON.stringify({
        v: 1,
        baseUrl: "http://127.0.0.1:4399",
        wsBaseUrl: "ws://127.0.0.1:4399",
      }),
    );

    const mod = await import("./daemonConnection");
    const connection = mod.getDaemonConnection();

    expect(connection.baseUrl).toBe("http://127.0.0.1:4399");
    expect(connection.wsBaseUrl).toBe("ws://127.0.0.1:4399");
    expect(connection.authToken).toBeNull();
    expect(sessionStorage.getItem(SESSION_CONNECTION_KEY)).toContain('"v":1');
  });

  it("notifies subscribers only when values change", async () => {
    const mod = await import("./daemonConnection");
    const listener = vi.fn();
    const unsubscribe = mod.subscribeDaemonConnection(listener);

    mod.setDaemonConnection({ baseUrl: "http://127.0.0.1:4399", authToken: "abc" });
    expect(listener).toHaveBeenCalledTimes(1);
    expect(listener.mock.calls[0][0]).toMatchObject({
      baseUrl: "http://127.0.0.1:4399",
      wsBaseUrl: "ws://127.0.0.1:4399",
      authToken: "abc",
    });

    mod.setDaemonConnection({ baseUrl: "http://127.0.0.1:4399", authToken: "abc" });
    expect(listener).toHaveBeenCalledTimes(1);

    mod.setDaemonConnection({ authToken: "def" });
    expect(listener).toHaveBeenCalledTimes(2);

    unsubscribe();
  });

  it("clears session and persisted base keys atomically", async () => {
    const mod = await import("./daemonConnection");
    mod.setDaemonConnection(
      {
        baseUrl: "http://127.0.0.1:4399",
        authToken: "abc",
      },
      { persistBaseUrl: true },
    );

    expect(localStorage.getItem(LOCAL_PERSISTED_BASE_KEY)).toContain('"v":1');

    mod.clearDaemonConnection({ persistBaseUrl: true, clearPersistedBaseUrl: true });
    const connection = mod.getDaemonConnection();

    expect(connection.baseUrl).toBeNull();
    expect(connection.wsBaseUrl).toBeNull();
    expect(connection.authToken).toBeNull();
    expect(localStorage.getItem(LOCAL_PERSISTED_BASE_KEY)).toBeNull();
  });

  it("ignores legacy split keys when canonical state is absent", async () => {
    sessionStorage.setItem("ctxAuthToken", "legacy-token");
    localStorage.setItem("contextDaemonBaseUrl", "http://127.0.0.1:4399");

    const mod = await import("./daemonConnection");
    const connection = mod.getDaemonConnection();

    expect(connection.baseUrl).toBe(window.location.origin);
    expect(connection.wsBaseUrl).toBe(window.location.origin.replace(/^http/, "ws"));
    expect(connection.authToken).toBeNull();
    expect(sessionStorage.getItem(SESSION_CONNECTION_KEY)).toContain('"v":1');
  });

  it("seeds same-origin base automatically in browser mode", async () => {
    const mod = await import("./daemonConnection");
    const connection = mod.getDaemonConnection();

    expect(connection.baseUrl).toBe(window.location.origin);
    expect(connection.wsBaseUrl).toBe(window.location.origin.replace(/^http/, "ws"));
  });

  it("applies dev env daemon url even after same-origin preseed", async () => {
    vi.stubEnv("VITE_CTX_DAEMON_URL", "http://127.0.0.1:4399");
    const mod = await import("./daemonConnection");

    // Ensure preseed happened.
    expect(mod.getDaemonConnection().baseUrl).toBe(window.location.origin);

    mod.bootstrapDaemonConnectionFromRuntime();
    const connection = mod.getDaemonConnection();
    expect(connection.baseUrl).toBe("http://127.0.0.1:4399");
    expect(connection.wsBaseUrl).toBe("ws://127.0.0.1:4399");
  });

  it("refreshes stale stored auth token from dev env", async () => {
    vi.stubEnv("VITE_CTX_AUTH_TOKEN", "fresh-token");
    const mod = await import("./daemonConnection");
    mod.setDaemonConnection({ authToken: "stale-token", source: "test" });
    expect(mod.getDaemonConnection().authToken).toBe("stale-token");

    mod.bootstrapDaemonConnectionFromRuntime();
    expect(mod.getDaemonConnection().authToken).toBe("fresh-token");
  });

  it("keeps URL token precedence over dev env token", async () => {
    vi.stubEnv("VITE_CTX_AUTH_TOKEN", "env-token");
    const mod = await import("./daemonConnection");
    window.history.replaceState({}, "", "/?token=url-token");

    mod.bootstrapDaemonConnectionFromRuntime();
    const connection = mod.getDaemonConnection();
    expect(connection.authToken).toBe("url-token");
    expect(window.location.search).toBe("");
  });
});
