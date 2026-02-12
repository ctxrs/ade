import { test as base, expect, chromium } from "playwright/test";

const AUTH_TOKEN = process.env.CTX_E2E_AUTH_TOKEN ?? "ctx-e2e-auth-token";

const test = base.extend({
  context: async ({ context }, use) => {
    await context.addInitScript((token: string) => {
      const wsUrls: string[] = [];
      const OriginalWebSocket = window.WebSocket;
      if (typeof OriginalWebSocket === "function") {
        class TrackedWebSocket extends OriginalWebSocket {
          constructor(url: string | URL, protocols?: string | string[]) {
            wsUrls.push(String(url));
            super(url, protocols as string | string[] | undefined);
          }
        }
        for (const key of Object.getOwnPropertyNames(OriginalWebSocket)) {
          if (Object.prototype.hasOwnProperty.call(TrackedWebSocket, key)) continue;
          try {
            Object.defineProperty(TrackedWebSocket, key, {
              value: (OriginalWebSocket as any)[key],
              configurable: true,
              writable: true,
            });
          } catch {
            // ignore define errors
          }
        }
        (window as any).WebSocket = TrackedWebSocket as typeof WebSocket;
      }
      window.sessionStorage.setItem(
        "ctxDaemonConnectionV1",
        JSON.stringify({
          v: 1,
          baseUrl: window.location.origin,
          wsBaseUrl: window.location.origin.replace(/^http/, "ws"),
          authToken: token,
          source: "e2e_init",
        }),
      );
      // Used by a few tests to enable app-side E2E hooks (disabled in normal usage).
      window.sessionStorage.setItem("ctxE2E", "1");
      (window as any).__ctxE2E ??= {};
      (window as any).__ctxE2E.getOpenedWebSocketUrls = () => wsUrls.slice();
      (window as any).__ctxE2E.clearOpenedWebSocketUrls = () => {
        wsUrls.length = 0;
      };
    }, AUTH_TOKEN);
    await use(context);
  },
});

export { test, expect, chromium };
