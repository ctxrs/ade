const DAEMON_JSON_RETRY_ATTEMPTS = 4;
const DAEMON_JSON_RETRY_BASE_DELAY_MS = 250;
const DAEMON_HTTP_TIMEOUT_MS = Number.parseInt(
  String(process.env.CTX_AUTOMATION_DAEMON_HTTP_TIMEOUT_MS || "60000"),
  10,
) || 60000;
let cachedConnection = null;
const waitMs = (ms) => new Promise((resolve) => setTimeout(resolve, Math.max(0, Number(ms) || 0)));

const connectionSignature = (connection) => JSON.stringify({
  kind: connection?.kind || null,
  base_url: connection?.base_url || null,
  token: connection?.token || null,
  host: connection?.host || null,
  user: connection?.user || null,
  remote_port: connection?.remote_port ?? null,
  remote_data_dir: connection?.remote_data_dir || null,
});

const shouldRetryWebdriverTransportError = (error) => {
  const text = String(error || "");
  return (
    text.includes("WebDriverError: Request failed with error code EADDRNOTAVAIL")
    || text.includes("WebDriverError: Request failed with error code ECONNREFUSED")
    || text.includes("WebDriverError: The operation was aborted due to timeout")
    || text.includes("Websocket connection lost")
    || text.includes("socket hang up")
  );
};

const getDesktopConnection = async () => {
  const invokeDesktop = async (cmd) => {
    const result = await browser.execute(async (command) => {
      const invoke = window.__TAURI__?.core?.invoke;
      if (!invoke) return { error: "Tauri invoke not available" };
      try {
        const info = await invoke(command);
        return { info };
      } catch (e) {
        return { error: String(e) };
      }
    }, cmd);
    if (result && typeof result === "object" && result.error) {
      throw new Error(result.error);
    }
    return result?.info || null;
  };

  const result = await browser.execute(async () => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return { error: "Tauri invoke not available" };
    try {
      const info = await invoke("desktop_get_connection");
      return { info };
    } catch (e) {
      return { error: String(e) };
    }
  });
  if (result && typeof result === "object" && result.error) {
    throw new Error(result.error);
  }
  const info = result.info || null;
  if (!info || typeof info !== "object") {
    throw new Error("desktop_get_connection returned no connection info");
  }

  if (info.base_url && info.token) {
    return info;
  }

  if (info.kind === "none" || info.kind === "local") {
    await invokeDesktop("desktop_connect_local");
    const refreshed = await invokeDesktop("desktop_get_connection");
    if (refreshed && refreshed.base_url && refreshed.token) {
      return refreshed;
    }
    throw new Error(
      `desktop connection missing base_url/token after desktop_connect_local: ${JSON.stringify(refreshed || info)}`,
    );
  }

  throw new Error(`desktop_get_connection missing base_url/token: ${JSON.stringify(info)}`);
};

const getCachedDesktopConnection = async () => {
  const current = await getDesktopConnection();

  if (cachedConnection && connectionSignature(cachedConnection) === connectionSignature(current)) {
    return cachedConnection;
  }

  cachedConnection = current;
  return cachedConnection;
};

const daemonHttpJson = async (connection, method, apiPath, body) => {
  const base = String(connection.base_url || "");
  const token = String(connection.token || "");
  if (!base || !token) {
    throw new Error(`daemon connection missing base_url/token: ${JSON.stringify(connection || null)}`);
  }
  if (typeof fetch !== "function") {
    throw new Error("global fetch is not available in this Node runtime");
  }
  const url = new URL(apiPath, base).toString();
  const headers = {
    authorization: `Bearer ${token}`,
    "content-type": "application/json",
  };
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(new Error("daemon request timeout")), DAEMON_HTTP_TIMEOUT_MS);
  try {
    const response = await fetch(url, {
      method,
      headers,
      body: typeof body === "undefined" ? undefined : JSON.stringify(body),
      signal: controller.signal,
    });
    const raw = await response.text();
    let payload = {};
    if (raw && raw.trim()) {
      try {
        payload = JSON.parse(raw);
      } catch {
        payload = { raw };
      }
    }
    return { status: response.status, payload };
  } finally {
    clearTimeout(timeout);
  }
};

const shouldRetryDaemonHttpError = (error) => {
  const text = String(error || "");
  return (
    text.includes("ECONNREFUSED")
    || text.includes("socket hang up")
    || text.includes("fetch failed")
    || text.includes("aborted")
    || text.includes("ETIMEDOUT")
    || text.includes("EADDRNOTAVAIL")
  );
};

const daemonJson = async (method, apiPath, body) => {
  let connection = await getCachedDesktopConnection();
  let lastError = null;
  for (let attempt = 1; attempt <= DAEMON_JSON_RETRY_ATTEMPTS; attempt += 1) {
    try {
      const resp = await daemonHttpJson(connection, method, apiPath, body);
      if (Number(resp.status) === 401 || Number(resp.status) === 403) {
        cachedConnection = null;
        connection = await getCachedDesktopConnection();
        return await daemonHttpJson(connection, method, apiPath, body);
      }
      if (attempt > 1) {
        cachedConnection = connection;
      }
      return resp;
    } catch (error) {
      lastError = error;
      const retryableTransport =
        shouldRetryWebdriverTransportError(error) || shouldRetryDaemonHttpError(error);
      if (
        attempt >= DAEMON_JSON_RETRY_ATTEMPTS
        || !retryableTransport
      ) {
        throw error;
      }
      // SSH reconnects can change desktop_get_connection.base_url (new tunnel/port).
      // On transport errors, refresh cached connection before retrying.
      cachedConnection = null;
      try {
        connection = await getCachedDesktopConnection();
      } catch {
        // best-effort; backoff/retry will surface the original transport error if still broken
      }
      const backoff = DAEMON_JSON_RETRY_BASE_DELAY_MS * attempt;
      await waitMs(backoff);
    }
  }

  throw lastError || new Error("daemonJson request failed");
};

const safeDaemonJson = async (method, apiPath, body) => {
  try {
    return await daemonJson(method, apiPath, body);
  } catch (error) {
    return { error: String(error) };
  }
};

const checkDaemonHealth = async () => {
  const startedAtMs = Date.now();
  const health = await safeDaemonJson("GET", "/api/health");
  return {
    ts: new Date(startedAtMs).toISOString(),
    elapsed_ms: Date.now() - startedAtMs,
    ok: Number(health.status || 0) === 200 && !health.error,
    status: health.status ?? null,
    error: health.error || null,
    payload: health.payload || null,
  };
};

const sampleDaemonHealth = async ({ durationMs, intervalMs = 1000 }) => {
  const started = Date.now();
  const samples = [];
  while (Date.now() - started < durationMs) {
    samples.push(await checkDaemonHealth());
    await waitMs(intervalMs);
  }
  return {
    duration_ms: durationMs,
    interval_ms: intervalMs,
    ok: samples.every((s) => s.ok),
    failures: samples.filter((s) => !s.ok),
    samples,
  };
};

module.exports = {
  daemonJson,
  safeDaemonJson,
  getDesktopConnection,
  checkDaemonHealth,
  sampleDaemonHealth,
};
