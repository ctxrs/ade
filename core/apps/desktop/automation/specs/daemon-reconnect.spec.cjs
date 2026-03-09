const { waitForTauri, getConnectionInfo } = require("./helpers/tauri.cjs");
const { daemonJson } = require("./helpers/daemon.cjs");
const { assertDesktopConnectionStable } = require("./helpers/workspace_wizard_flow.cjs");

const connectLocal = async () => {
  const result = await browser.execute(async () => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return { error: "Tauri invoke not available" };
    try {
      const info = await invoke("desktop_connect_local");
      return { info };
    } catch (e) {
      return { error: String(e) };
    }
  });
  if (result && typeof result === "object" && result.error) {
    throw new Error(result.error);
  }
  return result.info;
};

const disconnectDesktop = async () => {
  const result = await browser.execute(async () => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return { error: "Tauri invoke not available" };
    try {
      await invoke("desktop_disconnect");
      return { ok: true };
    } catch (e) {
      return { error: String(e) };
    }
  });
  if (result && typeof result === "object" && result.error) {
    throw new Error(result.error);
  }
};

const requestDeepLinkToken = async () => {
  const result = await browser.execute(async () => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return { error: "Tauri invoke not available" };
    try {
      const token = await invoke("desktop_get_deep_link_token");
      return { token };
    } catch (e) {
      return { error: String(e) };
    }
  });
  if (result && typeof result === "object" && result.error) {
    throw new Error(result.error);
  }
  return result.token;
};

describe("desktop daemon reconnect + deep-link token", () => {
  it("auto-reconnects on daemon request after disconnect and returns deep-link token", async () => {
    await browser.url("tauri://localhost");
    await waitForTauri();

    const first = await connectLocal();
    if (!first || first.kind !== "local") {
      throw new Error(`expected local connection after desktop_connect_local, got: ${JSON.stringify(first)}`);
    }

    const health1 = await daemonJson("GET", "/api/health");
    if (health1.status !== 200) {
      throw new Error(`initial /api/health failed: ${JSON.stringify(health1)}`);
    }

    await disconnectDesktop();
    const afterDisconnect = await getConnectionInfo();
    if (!afterDisconnect || afterDisconnect.kind !== "none") {
      throw new Error(`expected disconnected state, got: ${JSON.stringify(afterDisconnect)}`);
    }

    // desktop_daemon_request should auto-connect local on demand.
    const health2 = await daemonJson("GET", "/api/health");
    if (health2.status !== 200) {
      throw new Error(`post-disconnect /api/health failed: ${JSON.stringify(health2)}`);
    }

    const afterReconnect = await getConnectionInfo();
    if (!afterReconnect || afterReconnect.kind !== "local") {
      throw new Error(`expected auto-reconnected local state, got: ${JSON.stringify(afterReconnect)}`);
    }
    if (!afterReconnect.base_url || !afterReconnect.token) {
      throw new Error(`expected base_url + token after reconnect, got: ${JSON.stringify(afterReconnect)}`);
    }

    for (let attempt = 0; attempt < 3; attempt += 1) {
      const health = await daemonJson("GET", "/api/health");
      if (health.status !== 200) {
        throw new Error(`reconnect stability /api/health failed on attempt ${attempt + 1}: ${JSON.stringify(health)}`);
      }
    }
    await assertDesktopConnectionStable(5000, 250);
    const token = await requestDeepLinkToken();
    if (!token || typeof token.token !== "string" || token.token.trim().length < 8) {
      throw new Error(`invalid deep link token payload: ${JSON.stringify(token)}`);
    }
    if (!Number.isFinite(token.expires_at_ms) || token.expires_at_ms <= Date.now()) {
      throw new Error(`deep link token expiry must be in the future: ${JSON.stringify(token)}`);
    }
  });
});
