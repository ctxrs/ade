const DAEMON_JSON_RETRY_ATTEMPTS = 4;
const DAEMON_JSON_RETRY_BASE_DELAY_MS = 250;
const DAEMON_HTTP_TIMEOUT_MS = Number.parseInt(
  String(process.env.CTX_AUTOMATION_DAEMON_HTTP_TIMEOUT_MS || "60000"),
  10,
) || 60000;
const REMOTE_FIXTURE_DIRECT_DAEMON = String(
  process.env.CTX_AUTOMATION_REMOTE_DIRECT_DAEMON || "0",
).trim() === "1";
const REMOTE_FIXTURE_DIRECT_DAEMON_URL = String(
  process.env.CTX_AUTOMATION_REMOTE_DIRECT_DAEMON_URL || "",
).trim();
const REMOTE_FIXTURE_HOST = String(
  process.env.CTX_AUTOMATION_REMOTE_HOST || process.env.CTX_AUTOMATION_REMOTE_CONTAINER_HOST || "",
).trim();
const REMOTE_FIXTURE_PORT = Number.parseInt(
  String(process.env.CTX_AUTOMATION_REMOTE_PORT || process.env.CTX_AUTOMATION_REMOTE_CONTAINER_PORT || ""),
  10,
) || 0;
let cachedConnection = null;
const waitMs = (ms) => new Promise((resolve) => setTimeout(resolve, Math.max(0, Number(ms) || 0)));

const resolveDaemonAuthPath = () => {
  const explicit = readString(process.env.CTX_AUTOMATION_DAEMON_AUTH_PATH);
  if (explicit) return explicit;
  const shippedDataDir = readString(process.env.CTX_AUTOMATION_SHIPPED_APP_DAEMON_DATA_DIR);
  if (shippedDataDir) return path.join(shippedDataDir, "daemon_auth.json");
  return path.join(os.homedir(), ".ctx", "daemon_auth.json");
};

const shouldRefreshLocalDesktopConnection = (info) => {
  if (!info || typeof info !== "object") return false;
  return String(info.kind || "").trim().toLowerCase() === "local";
};

const connectionSignature = (connection) => JSON.stringify({
  kind: connection?.kind || null,
  base_url: connection?.base_url || null,
  browser_query_secret: connection?.browser_query_secret || null,
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

const maybeDirectRemoteDaemonConnection = (connection) => {
  if (!REMOTE_FIXTURE_DIRECT_DAEMON) return connection;
  if (!connection || typeof connection !== "object") return connection;
  const kind = String(connection.kind || "").trim().toLowerCase();
  if (kind !== "ssh") return connection;
  if (REMOTE_FIXTURE_DIRECT_DAEMON_URL) {
    return {
      ...connection,
      base_url: REMOTE_FIXTURE_DIRECT_DAEMON_URL,
    };
  }
  if (!REMOTE_FIXTURE_HOST || REMOTE_FIXTURE_PORT <= 0) return connection;
  return {
    ...connection,
    base_url: `http://${REMOTE_FIXTURE_HOST}:${REMOTE_FIXTURE_PORT}`,
  };
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

  if (info.base_url && info.browser_query_secret) {
    if (shouldRefreshLocalDesktopConnection(info)) {
      const refreshed = await invokeDesktop("desktop_connect_local");
      if (refreshed && refreshed.base_url && refreshed.browser_query_secret) {
        return maybeDirectRemoteDaemonConnection(refreshed);
      }
      throw new Error(
        `desktop_connect_local returned no browser-scoped connection info after stale/local refresh: ${JSON.stringify(refreshed || info)}`,
      );
    }
    return maybeDirectRemoteDaemonConnection(info);
  }

  if (info.kind === "none" || info.kind === "local") {
    await invokeDesktop("desktop_connect_local");
    const refreshed = await invokeDesktop("desktop_get_connection");
    if (refreshed && refreshed.base_url && refreshed.browser_query_secret) {
      return maybeDirectRemoteDaemonConnection(refreshed);
    }
    throw new Error(
      `desktop connection missing base_url/browser_query_secret after desktop_connect_local: ${JSON.stringify(refreshed || info)}`,
    );
  }

  throw new Error(`desktop_get_connection missing base_url/browser_query_secret: ${JSON.stringify(info)}`);
};

const getCachedDesktopConnection = async () => {
  if (cachedConnection) {
    return cachedConnection;
  }

  cachedConnection = await getDesktopConnection();
  return cachedConnection;
};

const daemonHttpJson = async (connection, method, apiPath, body) => {
  const base = String(connection.base_url || "");
  const browserQuerySecret = String(connection.browser_query_secret || "");
  if (!base || !browserQuerySecret) {
    throw new Error(`daemon connection missing base_url/browser_query_secret: ${JSON.stringify(connection || null)}`);
  }
  if (typeof fetch !== "function") {
    throw new Error("global fetch is not available in this Node runtime");
  }
  const url = new URL(apiPath, base).toString();
  const headers = {
    authorization: `Bearer ${browserQuerySecret}`,
    "content-type": "application/json",
  };
  const controller = new AbortController();
  const timeout = setTimeout(() => {
    controller.abort(new Error(`daemon request timeout: ${method} ${apiPath}`));
  }, DAEMON_HTTP_TIMEOUT_MS);
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

const daemonJsonOnce = async (method, apiPath, body) => {
  let connection = await getCachedDesktopConnection();
  let response;
  try {
    response = await daemonHttpJson(connection, method, apiPath, body);
  } catch (error) {
    const retryableTransport =
      shouldRetryWebdriverTransportError(error) || shouldRetryDaemonHttpError(error);
    if (!retryableTransport) {
      throw error;
    }
    cachedConnection = null;
    connection = await getCachedDesktopConnection();
    return await daemonHttpJson(connection, method, apiPath, body);
  }
  if (Number(response.status) === 401 || Number(response.status) === 403) {
    cachedConnection = null;
    connection = await getCachedDesktopConnection();
    return await daemonHttpJson(connection, method, apiPath, body);
  }
  return response;
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

const readString = (value) => (typeof value === "string" ? value.trim() : "");

const asRecord = (value) => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value;
};

const expectedDaemonIdentityFromEnv = ({ env = process.env } = {}) => ({
  version: readString(env.CTX_AUTOMATION_EXPECT_DAEMON_VERSION),
  buildId: readString(env.CTX_AUTOMATION_EXPECT_DAEMON_BUILD_ID),
  compatibilityToken: readString(env.CTX_AUTOMATION_EXPECT_DAEMON_COMPATIBILITY_TOKEN),
});

const daemonIdentityFromHealthPayload = (payload) => {
  const body = asRecord(payload);
  const compatibility = asRecord(body.compatibility);
  return {
    version: readString(body.daemon_version || body.version),
    buildId: readString(compatibility.desktop_build_id),
    compatibilityToken: readString(compatibility.protocol_compatibility_token),
    pid: body.pid ?? null,
    dataRoot: readString(body.data_root),
    daemonUrl: readString(body.daemon_url),
  };
};

const daemonHealthWithRawAuth = async () => {
  const authPath = resolveDaemonAuthPath();
  let parsed;
  try {
    parsed = JSON.parse(fs.readFileSync(authPath, "utf8"));
  } catch (error) {
    throw new Error(`failed to read daemon auth for identity proof at ${authPath}: ${String(error)}`);
  }
  const baseUrl = readString(parsed.daemon_url || parsed.base_url);
  const token = readString(parsed.token);
  if (!baseUrl || !token) {
    throw new Error(`daemon auth for identity proof is missing daemon_url/token at ${authPath}`);
  }
  if (typeof fetch !== "function") {
    throw new Error("global fetch is not available in this Node runtime");
  }
  const controller = new AbortController();
  const timeout = setTimeout(() => {
    controller.abort(new Error("raw-auth daemon health request timeout"));
  }, DAEMON_HTTP_TIMEOUT_MS);
  try {
    const response = await fetch(new URL("/api/health", baseUrl).toString(), {
      method: "GET",
      headers: {
        authorization: `Bearer ${token}`,
      },
      signal: controller.signal,
    });
    const raw = await response.text();
    let payload = {};
    if (raw.trim()) {
      try {
        payload = JSON.parse(raw);
      } catch {
        payload = { raw };
      }
    }
    return {
      status: response.status,
      payload,
    };
  } finally {
    clearTimeout(timeout);
  }
};

const compareDaemonIdentity = (expected, actual) => {
  const mismatches = [];
  for (const [field, label] of [
    ["version", "daemon version"],
    ["buildId", "daemon build id"],
    ["compatibilityToken", "daemon compatibility token"],
  ]) {
    if (!expected[field]) continue;
    if (actual[field] !== expected[field]) {
      mismatches.push({
        field,
        label,
        expected: expected[field],
        actual: actual[field] || "",
      });
    }
  }
  return mismatches;
};

const assertExpectedDaemonIdentity = async ({
  expected = expectedDaemonIdentityFromEnv(),
  label = "daemon_identity",
} = {}) => {
  const normalizedExpected = {
    version: readString(expected.version),
    buildId: readString(expected.buildId),
    compatibilityToken: readString(expected.compatibilityToken),
  };
  const health = await daemonJson("GET", "/api/health");
  if (Number(health.status || 0) !== 200) {
    throw new Error(`${label}: expected /api/health 200 before product proof, got ${JSON.stringify(health)}`);
  }
  let actual = daemonIdentityFromHealthPayload(health.payload);
  let rawHealth = null;
  if (normalizedExpected.compatibilityToken && !actual.compatibilityToken) {
    rawHealth = await daemonHealthWithRawAuth();
    if (Number(rawHealth.status || 0) !== 200) {
      throw new Error(`${label}: expected raw-auth /api/health 200 before product proof, got status=${rawHealth.status}`);
    }
    actual = daemonIdentityFromHealthPayload(rawHealth.payload);
  }
  const expectedAny = Boolean(
    normalizedExpected.version
    || normalizedExpected.buildId
    || normalizedExpected.compatibilityToken,
  );
  const mismatches = expectedAny ? compareDaemonIdentity(normalizedExpected, actual) : [];
  const result = {
    label,
    skipped: !expectedAny,
    expected: normalizedExpected,
    actual,
    raw_auth_health_used: Boolean(rawHealth),
    mismatches,
  };
  if (mismatches.length > 0) {
    throw new Error(`${label}: daemon identity mismatch before product proof: ${JSON.stringify(result)}`);
  }
  return result;
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
  daemonJsonOnce,
  safeDaemonJson,
  getDesktopConnection,
  checkDaemonHealth,
  sampleDaemonHealth,
  expectedDaemonIdentityFromEnv,
  daemonIdentityFromHealthPayload,
  daemonHealthWithRawAuth,
  compareDaemonIdentity,
  assertExpectedDaemonIdentity,
  shouldRefreshLocalDesktopConnection,
};
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
