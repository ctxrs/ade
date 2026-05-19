const { navigateToTauriUrl } = require("./helpers/tauri.cjs");
const {
  tauriInvoke,
  waitForDesktopAppReady,
} = require("./helpers/container_lifecycle.cjs");

const requireTauriValue = async (command, payload = {}) => {
  const response = await tauriInvoke(command, payload);
  if (response.error) {
    throw new Error(`${command} failed: ${response.error}`);
  }
  return response.value;
};

const installTaskRouteListener = async () => {
  const installed = await browser.executeAsync((done) => {
    Promise.resolve()
      .then(async () => {
        window.__ctxTaskRouteEvents = [];
        const listen = window.__TAURI__?.event?.listen;
        if (typeof listen !== "function") {
          throw new Error("Tauri event listen API not available");
        }
        if (typeof window.__ctxTaskRouteUnlisten === "function") {
          window.__ctxTaskRouteUnlisten();
        }
        window.__ctxTaskRouteUnlisten = await listen("desktop_task_deeplink_open", (event) => {
          window.__ctxTaskRouteEvents.push({
            at: performance.now(),
            payload: event.payload,
          });
        });
        done({ ok: true });
      })
      .catch((error) => done({ error: String(error) }));
  });
  if (!installed?.ok) {
    throw new Error(installed?.error || "failed to install task route listener");
  }
};

const readRouteProbe = async () =>
  browser.execute(() => ({
    events: Array.isArray(window.__ctxTaskRouteEvents) ? window.__ctxTaskRouteEvents : [],
    href: window.location.href,
    startup: window.__CTX_DESKTOP_STARTUP__ || null,
  }));

describe("notification task routing", () => {
  it("routes simulated notification clicks to the existing webview without reload", async () => {
    await navigateToTauriUrl("tauri://localhost");
    await waitForDesktopAppReady();
    await requireTauriValue("desktop_connect_local");
    await requireTauriValue("desktop_clear_notification_automation_snapshot");
    await installTaskRouteListener();

    const nonce = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
    const workspaceId = `workspace-${nonce}`;
    const taskId = `task-${nonce}`;
    const sessionId = `session-${nonce}`;
    await requireTauriValue("desktop_record_workbench_route", {
      req: {
        active_session_id: sessionId,
        active_task_id: taskId,
        open_tasks: [{ task_id: taskId, session_id: sessionId }],
        workspace_id: workspaceId,
        workspace_label: `Workspace ${nonce}`,
      },
    });

    const before = await readRouteProbe();
    const startedAt = Date.now();
    await requireTauriValue("desktop_show_system_notification", {
      req: {
        kind: "turn_completed",
        title: `ctx notification route ${nonce}`,
        body: `Notification route ${nonce}`,
        workspace_id: workspaceId,
        task_id: taskId,
        session_id: sessionId,
      },
    });
    await requireTauriValue("desktop_simulate_last_notification_click");

    let lastProbe = null;
    await browser.waitUntil(async () => {
      lastProbe = await readRouteProbe();
      return lastProbe.events.some((event) => event?.payload?.task_id === taskId);
    }, {
      timeout: 5000,
      interval: 100,
      timeoutMsg: "notification click did not emit desktop task route event",
    });
    const latencyMs = Date.now() - startedAt;
    const after = lastProbe || await readRouteProbe();
    const matching = after.events.find((event) => event?.payload?.task_id === taskId);

    if (matching?.payload?.workspace_id !== workspaceId || matching?.payload?.session_id !== sessionId) {
      throw new Error(`unexpected route payload: ${JSON.stringify(matching?.payload || null)}`);
    }
    if (after.startup?.windowCreatedAtMs !== before.startup?.windowCreatedAtMs) {
      throw new Error("notification task route recreated or reloaded the webview");
    }
    if (after.href !== before.href) {
      throw new Error(`notification task route changed window location from ${before.href} to ${after.href}`);
    }
    console.log(`notification route latency ms=${latencyMs}`);
  });
});
