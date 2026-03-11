import { test as base, expect, chromium } from "playwright/test";

const AUTH_TOKEN = process.env.CTX_E2E_AUTH_TOKEN ?? "ctx-e2e-auth-token";
const FIXTURE_ISO = "2026-03-11T00:00:00.000Z";

type E2EWindow = Window & {
  __ctxE2E?: {
    getOpenedWebSocketUrls?: () => string[];
    clearOpenedWebSocketUrls?: () => void;
  };
};

const isRecord = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === "object" && !Array.isArray(value);

const asRecord = (value: unknown): Record<string, unknown> =>
  (isRecord(value) ? value : {});

const asArray = (value: unknown): unknown[] => (Array.isArray(value) ? value : []);

const mergeFakeHarnessBootstrap = (raw: unknown, workspaceId: string) => {
  const payload = asRecord(raw);
  const providers = asArray(payload.providers)
    .map((entry) => asRecord(entry))
    .filter((entry) => Object.keys(entry).length > 0);
  const fakeProviderIndex = providers.findIndex((entry) => entry.provider_id === "fake");
  const fakeProvider = {
    ...(fakeProviderIndex >= 0 ? providers[fakeProviderIndex] : {}),
    provider_id: "fake",
    installed: true,
    health: "ok",
    diagnostics: [],
    details: {
      ...asRecord(fakeProviderIndex >= 0 ? asRecord(providers[fakeProviderIndex]).details : undefined),
      ready_for_use: "true",
    },
  };
  if (fakeProviderIndex >= 0) {
    providers[fakeProviderIndex] = fakeProvider;
  } else {
    providers.push(fakeProvider);
  }

  const providerOptions = {
    ...asRecord(payload.provider_options),
    fake: {
      ...asRecord(asRecord(payload.provider_options).fake),
      provider_id: "fake",
      workspace_id: workspaceId,
      installed: true,
      supports_load: false,
      auth_required: false,
      has_active_auth: true,
      auth_mode: "subscription",
      probed_at: FIXTURE_ISO,
    },
  };

  const providerHarnessConfig = {
    ...asRecord(payload.provider_harness_config),
    fake: {
      provider_id: "fake",
      selected_source_kind: "subscription",
      selected_endpoint_id: null,
      endpoints: [],
    },
  };

  return {
    ...payload,
    providers,
    provider_options: providerOptions,
    provider_harness_config: providerHarnessConfig,
  };
};

const test = base.extend({
  context: async ({ context }, use) => {
    await context.route("**/api/workspaces/*/providers/bootstrap", async (route) => {
      if (route.request().method() !== "GET") {
        await route.continue();
        return;
      }
      const response = await route.fetch();
      if (!response.ok()) {
        await route.fulfill({ response });
        return;
      }
      const url = new URL(route.request().url());
      const match = url.pathname.match(/^\/api\/workspaces\/([^/]+)\/providers\/bootstrap$/);
      const workspaceId = match ? decodeURIComponent(match[1]) : "";
      const payload = mergeFakeHarnessBootstrap(await response.json(), workspaceId);
      await route.fulfill({
        status: response.status(),
        contentType: "application/json",
        body: JSON.stringify(payload),
      });
    });

    await context.route("**/api/workspaces/*/providers/fake/options", async (route) => {
      if (route.request().method() !== "GET") {
        await route.continue();
        return;
      }
      const url = new URL(route.request().url());
      const match = url.pathname.match(/^\/api\/workspaces\/([^/]+)\/providers\/fake\/options$/);
      const workspaceId = match ? decodeURIComponent(match[1]) : "";
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          provider_id: "fake",
          workspace_id: workspaceId,
          supports_load: false,
          auth_required: false,
          has_active_auth: true,
          auth_mode: "subscription",
          probed_at: new Date().toISOString(),
        }),
      });
    });

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
              value: Reflect.get(OriginalWebSocket as object, key),
              configurable: true,
              writable: true,
            });
          } catch {
            // ignore define errors
          }
        }
        Object.defineProperty(window, "WebSocket", {
          value: TrackedWebSocket,
          configurable: true,
          writable: true,
        });
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
      const w = window as E2EWindow;
      w.__ctxE2E ??= {};
      w.__ctxE2E.getOpenedWebSocketUrls = () => wsUrls.slice();
      w.__ctxE2E.clearOpenedWebSocketUrls = () => {
        wsUrls.length = 0;
      };
    }, AUTH_TOKEN);
    await use(context);
  },
});

export { test, expect, chromium };
