const { navigateToTauriUrl } = require("./helpers/tauri.cjs");
const {
  tauriInvoke,
  waitForDesktopAppReady,
} = require("./helpers/container_lifecycle.cjs");

const RECOVERY_SNACKBAR_SELECTOR = '[data-testid="desktop-webview-recovery-snackbar"]';

const getRecoverySnapshot = async () => {
  const response = await tauriInvoke("desktop_get_webview_recovery_automation_snapshot", {});
  if (response.error) {
    throw new Error(`desktop_get_webview_recovery_automation_snapshot failed: ${response.error}`);
  }
  return response.value || { windows: [], pending_incident_count: 0 };
};

const getMainWindowSnapshot = async () => {
  const snapshot = await getRecoverySnapshot();
  const mainWindow = Array.isArray(snapshot.windows)
    ? snapshot.windows.find((window) => window.window_label === "main")
    : null;
  if (!mainWindow) {
    throw new Error(`main recovery snapshot missing: ${JSON.stringify(snapshot)}`);
  }
  return mainWindow;
};

const waitForRoute = async (expectedRoute, timeoutMs = 30000) => {
  await browser.waitUntil(
    async () => {
      try {
        const current = await browser.execute(
          () => `${window.location.pathname}${window.location.search}${window.location.hash}`,
        );
        return current === expectedRoute;
      } catch {
        return false;
      }
    },
    { timeout: timeoutMs, interval: 250, timeoutMsg: `expected route ${expectedRoute}` },
  );
};

const waitForMainWindowHealthy = async (expectedRoute, timeoutMs = 30000) => {
  await browser.waitUntil(
    async () => {
      try {
        const window = await getMainWindowSnapshot();
        return (
          window.route === expectedRoute
          && window.window_surface === "main"
          && !window.recovery_in_progress
          && !window.pending_heartbeat_timeout
          && Number(window.startup_completed_at_ms || 0) > 0
        );
      } catch {
        return false;
      }
    },
    {
      timeout: timeoutMs,
      interval: 500,
      timeoutMsg: `main recovery controller never stabilized for ${expectedRoute}`,
    },
  );
};

const waitForSnackbarText = async (expectedSnippet, timeoutMs = 30000) => {
  await browser.waitUntil(
    async () => {
      try {
        const text = await browser.execute((selector) => {
          const node = document.querySelector(selector);
          return node ? String(node.textContent || "") : "";
        }, RECOVERY_SNACKBAR_SELECTOR);
        return text.includes(expectedSnippet);
      } catch {
        return false;
      }
    },
    {
      timeout: timeoutMs,
      interval: 250,
      timeoutMsg: `desktop recovery snackbar did not contain: ${expectedSnippet}`,
    },
  );
};

const openSettingsRoute = async () => {
  const nonce = Date.now();
  const route = `/settings?recoveryE2E=${nonce}`;
  await browser.execute((targetRoute) => {
    window.location.href = targetRoute;
  }, route);
  await waitForRoute(route);
  await waitForMainWindowHealthy(route);
  return route;
};

const injectRecoveryFault = async (kind) => {
  await browser.execute((faultKind) => {
    const invoke = window.__TAURI_INTERNALS__?.invoke || window.__TAURI__?.core?.invoke;
    if (!invoke) {
      throw new Error("desktop recovery fault injector missing Tauri invoke bridge");
    }
    void invoke("desktop_trigger_webview_recovery_fault", { req: { kind: faultKind } }).catch(() => {});
  }, kind);
};

describe("desktop webview recovery", () => {
  it("reloads then recreates the main webview while preserving the route", async () => {
    await navigateToTauriUrl("tauri://localhost");
    await waitForDesktopAppReady();

    const trackedRoute = await openSettingsRoute();

    await injectRecoveryFault("native_process_termination");
    await waitForDesktopAppReady();
    await waitForRoute(trackedRoute);
    await waitForMainWindowHealthy(trackedRoute);
    await waitForSnackbarText("ctx recovered a failed main window.");

    const firstSnapshot = await getMainWindowSnapshot();
    if (firstSnapshot.recent_incident_count !== 1) {
      throw new Error(`expected first recovery incident count to be 1: ${JSON.stringify(firstSnapshot)}`);
    }

    await injectRecoveryFault("native_process_termination");
    await waitForDesktopAppReady();
    await waitForRoute(trackedRoute);
    await waitForMainWindowHealthy(trackedRoute);
    await waitForSnackbarText("ctx reopened a failed main window.");

    const secondSnapshot = await getMainWindowSnapshot();
    if (secondSnapshot.recent_incident_count !== 2) {
      throw new Error(`expected second recovery incident count to be 2: ${JSON.stringify(secondSnapshot)}`);
    }
  });
});
