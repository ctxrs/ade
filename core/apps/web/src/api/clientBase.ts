import type { ClientTelemetryBatch, SemanticTelemetryBatch, SemanticTelemetryEvent } from "@ctx/types";
import { isDesktopApp } from "../utils/desktop";
import { emitUiDiagnostic, normalizeDiagnosticErrorMessage } from "../state/diagnosticsChannel";
import {
  ensureDesktopDaemonConnection,
  syncDesktopDaemonConnectionFromBridge as syncDesktopDaemonConnectionFromBridgeImpl,
} from "./desktopDaemonConnection";
import {
  applyDesktopDaemonConnection,
  bootstrapDaemonConnectionFromRuntime,
  clearDaemonConnection,
  getDaemonConnection,
  getDaemonHttpUrl,
  normalizeDaemonBaseUrl,
  setDaemonConnection,
  subscribeDaemonConnection,
} from "./daemonConnection";
import { buildDaemonRequestHeaders } from "./daemonRequestHeaders";

export type DaemonClientConfig = {
  baseUrl: string | null;
  wsBaseUrl: string | null;
  authToken: string | null;
  runId: string | null;
};

type DaemonConfigListener = (config: DaemonClientConfig) => void;
export const authToken = (): string | null => getDaemonConnection().authToken;

export const getDaemonClientConfig = (): DaemonClientConfig => {
  const connection = getDaemonConnection();
  return {
    baseUrl: connection.baseUrl,
    wsBaseUrl: connection.wsBaseUrl,
    authToken: connection.authToken,
    runId: getTelemetryRunId(),
  };
};

export const subscribeDaemonConfig = (listener: DaemonConfigListener): (() => void) =>
  subscribeDaemonConnection(() => listener(getDaemonClientConfig()));

export const setDaemonBaseUrl = (baseUrl: string | null, persist?: boolean) => {
  setDaemonConnection(
    { baseUrl: normalizeDaemonBaseUrl(baseUrl), source: "set_base_url" },
    { persistBaseUrl: Boolean(persist) },
  );
};

export const setDaemonAuthToken = (token: string | null) => {
  setDaemonConnection({ authToken: token, source: "set_auth_token" });
};

export const applyDaemonDesktopConnection = applyDesktopDaemonConnection;
export const resetDaemonConnection = clearDaemonConnection;
export const primeDaemonConnection = bootstrapDaemonConnectionFromRuntime;

export type DesktopDaemonConnectionSyncResult = {
  config: DaemonClientConfig;
  info: Awaited<ReturnType<typeof syncDesktopDaemonConnectionFromBridgeImpl>>["info"];
  synced: boolean;
  error: string | null;
};

type DesktopDaemonConnectionSyncOptions = {
  force?: boolean;
  probeHealth?: boolean;
  reason?: string;
};

const makeDesktopSyncResult = (
  info: Awaited<ReturnType<typeof syncDesktopDaemonConnectionFromBridgeImpl>>["info"],
  error: string | null,
): DesktopDaemonConnectionSyncResult => ({
  config: getDaemonClientConfig(),
  info,
  synced: Boolean(info),
  error,
});

export const syncDesktopDaemonConnectionFromBridge = async (
  opts?: DesktopDaemonConnectionSyncOptions,
): Promise<DesktopDaemonConnectionSyncResult> => {
  const result = await syncDesktopDaemonConnectionFromBridgeImpl({
    force: opts?.force,
    connectLocalWhenMissing: opts?.probeHealth,
    reason: opts?.reason,
  });
  return makeDesktopSyncResult(result.info, result.error);
};

const shouldEmitApiDiagnostic = (path: string): boolean =>
  path.startsWith("/api/") && !path.startsWith("/api/telemetry");

const emitApiDiagnostic = (args: {
  path: string;
  method: string;
  status?: number;
  code: "api.transport_error" | "api.http_error";
  severity: "error" | "warning";
  message: string;
}) => {
  if (!shouldEmitApiDiagnostic(args.path)) return;
  emitUiDiagnostic({
    source: "api",
    code: args.code,
    severity: args.severity,
    message: args.message,
    context: {
      path: args.path,
      method: args.method,
      status: args.status,
    },
  });
};

export const api = async <T>(path: string, init?: RequestInit): Promise<T> => {
  const token = authToken();
  const traceparent = createTraceparent();
  const runId = getTelemetryRunId();
  const method = init?.method ? String(init.method) : "GET";
  const start = typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
  let res: Response;
  try {
    res = await fetch(getDaemonHttpUrl(path), {
      ...init,
      headers: buildDaemonRequestHeaders({
        headers: init?.headers,
        token,
        traceparent,
        runId,
      }),
    });
  } catch (err) {
    const end = typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
    recordClientApiError(path, method, end - start, runId);
    emitApiDiagnostic({
      path,
      method,
      code: "api.transport_error",
      severity: "error",
      message: normalizeDiagnosticErrorMessage(err, "Request failed before receiving a response."),
    });
    throw err;
  }
  const end = typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
  recordClientApiMetric(path, method, res.status, res.status < 500, end - start, runId);

  const looksLikeHtml = (text: string): boolean => {
    const t = String(text || "").trimStart().toLowerCase();
    return t.startsWith("<!doctype html") || t.startsWith("<html");
  };

  const trimForError = (text: string): string => {
    const s = String(text || "").trim();
    if (s.length <= 800) return s;
    return `${s.slice(0, 800)}…`;
  };

  if (!res.ok) {
    const text = await res.text();
    const contentType = res.headers.get("content-type") ?? "";

    if ((contentType.includes("text/html") || looksLikeHtml(text)) && path.startsWith("/api/")) {
      // This usually means the web UI server served its SPA fallback for an /api route.
      // Most commonly: the daemon is old and doesn't implement the endpoint, or the dev proxy isn't pointing at the daemon.
      const message = `The daemon returned HTML for ${path} (${res.status}). Restart/update the daemon (and ensure Vite is proxying /api to it).`;
      emitApiDiagnostic({
        path,
        method,
        status: res.status,
        code: "api.http_error",
        severity: "error",
        message,
      });
      throw new Error(message);
    }

    const lowered = String(text || "").toLowerCase();
    if (
      res.status >= 500 &&
      (lowered.includes("econnrefused") ||
        lowered.includes("proxy error") ||
        lowered.includes("connect econnrefused") ||
        lowered.includes("socket hang up"))
    ) {
      const message =
        "Cannot reach the ctx daemon via /api. If you're running the web dev server, start the daemon (default http://127.0.0.1:4399) or set CTX_DAEMON_URL before `pnpm dev`.";
      emitApiDiagnostic({
        path,
        method,
        status: res.status,
        code: "api.http_error",
        severity: "error",
        message,
      });
      throw new Error(message);
    }
    let parsedMessage: string | null = null;
    try {
      const parsed = text ? JSON.parse(text) : null;
      const msg = parsed?.error ?? parsed?.message;
      if (typeof msg === "string" && msg.length > 0) {
        parsedMessage = msg;
      }
    } catch {
      // ignore
    }
    const message = parsedMessage ?? (trimForError(text) || `${res.status} ${res.statusText}`);
    emitApiDiagnostic({
      path,
      method,
      status: res.status,
      code: "api.http_error",
      severity: res.status >= 500 ? "error" : "warning",
      message,
    });
    throw new Error(message);
  }
  if (res.status === 204) {
    return undefined as T;
  }
  const text = await res.text();
  if (!text) return undefined as T;
  try {
    return JSON.parse(text) as T;
  } catch {
    const contentType = res.headers.get("content-type") ?? "";
    if ((contentType.includes("text/html") || looksLikeHtml(text)) && path.startsWith("/api/")) {
      throw new Error(
        `The daemon returned HTML for ${path}. Restart/update the daemon (and ensure Vite is proxying /api to it).`,
      );
    }
    throw new Error(`Unexpected non-JSON response from ${path}.`);
  }
};

const desktopApi = async <T>(path: string, init?: RequestInit): Promise<T> => {
  await ensureDesktopDaemonConnection({
    connectLocalWhenMissing: true,
    reason: "desktop_api_preflight",
  });
  const token = authToken();
  const traceparent = createTraceparent();
  const runId = getTelemetryRunId();

  const method = init?.method ? String(init.method) : "GET";

  const start = typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
  let res: Response;
  try {
    res = await fetch(getDaemonHttpUrl(path), {
      ...init,
      headers: buildDaemonRequestHeaders({
        headers: init?.headers,
        token,
        traceparent,
        runId,
      }),
    });
  } catch (err) {
    const end = typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
    recordClientApiError(path, method, end - start, runId);
    emitApiDiagnostic({
      path,
      method,
      code: "api.transport_error",
      severity: "error",
      message: normalizeDiagnosticErrorMessage(err, "Desktop daemon request failed."),
    });
    throw err;
  }
  const end = typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
  recordClientApiMetric(path, method, res.status, res.status < 500, end - start, runId);

  const contentType = String(res.headers.get("content-type") ?? "");
  const text = await res.text();

  const looksLikeHtml = (t: string): boolean => {
    const s = String(t || "").trimStart().toLowerCase();
    return s.startsWith("<!doctype html") || s.startsWith("<html");
  };

  const trimForError = (t: string): string => {
    const s = String(t || "").trim();
    if (s.length <= 800) return s;
    return `${s.slice(0, 800)}…`;
  };

  const ok = res.status >= 200 && res.status < 300;
  if (!ok) {
    if ((contentType.includes("text/html") || looksLikeHtml(text)) && path.startsWith("/api/")) {
      const message = `The daemon returned HTML for ${path} (${res.status}). Restart/update the daemon.`;
      emitApiDiagnostic({
        path,
        method,
        status: res.status,
        code: "api.http_error",
        severity: "error",
        message,
      });
      throw new Error(message);
    }
    const lowered = String(text || "").toLowerCase();
    if (
      res.status >= 500 &&
      (lowered.includes("econnrefused") ||
        lowered.includes("proxy error") ||
        lowered.includes("connect econnrefused") ||
        lowered.includes("socket hang up"))
    ) {
      const message = "Cannot reach the ctx daemon. Connect to a host from the launcher first.";
      emitApiDiagnostic({
        path,
        method,
        status: res.status,
        code: "api.http_error",
        severity: "error",
        message,
      });
      throw new Error(message);
    }
    let parsedMessage: string | null = null;
    try {
      const parsed = text ? JSON.parse(text) : null;
      const msg = parsed?.error ?? parsed?.message;
      if (typeof msg === "string" && msg.length > 0) {
        parsedMessage = msg;
      }
    } catch {
      // ignore
    }
    const message = parsedMessage ?? (trimForError(text) || `${res.status}`);
    emitApiDiagnostic({
      path,
      method,
      status: res.status,
      code: "api.http_error",
      severity: res.status >= 500 ? "error" : "warning",
      message,
    });
    throw new Error(message);
  }

  if (res.status === 204) {
    return undefined as T;
  }
  if (!text) return undefined as T;
  try {
    return JSON.parse(text) as T;
  } catch {
    if ((contentType.includes("text/html") || looksLikeHtml(text)) && path.startsWith("/api/")) {
      throw new Error(`The daemon returned HTML for ${path}. Restart/update the daemon.`);
    }
    throw new Error(`Unexpected non-JSON response from ${path}.`);
  }
};

export const apiAny = async <T>(path: string, init?: RequestInit): Promise<T> => {
  if (isDesktopApp()) return desktopApi<T>(path, init);
  return api<T>(path, init);
};

const CLIENT_TELEMETRY_PATH = "/api/telemetry/client";
const SEMANTIC_TELEMETRY_PATH = "/api/telemetry/events";
const CLIENT_TELEMETRY_FLUSH_MS = 1000;
const CLIENT_TELEMETRY_MAX = 200;
let clientTelemetryTimer: number | null = null;
const clientTelemetryQueue: ClientTelemetryMetric[] = [];
let semanticTelemetryTimer: number | null = null;
const semanticTelemetryQueue: SemanticTelemetryEvent[] = [];
let semanticTelemetryRemoteEnabled = true;

type ClientTelemetryMetric = {
  name: string;
  kind: "histogram" | "counter" | "gauge";
  unit: string;
  value: number;
  labels?: Record<string, string>;
  run_id?: string | null;
};

const normalizePath = (path: string): string => {
  const uuid = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi;
  const numeric = /\/(\d+)(?=\/|$)/g;
  return path.replace(uuid, ":id").replace(numeric, "/:id");
};

const toHex = (bytes: Uint8Array): string =>
  Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");

const createTraceparent = (): string | null => {
  if (typeof crypto === "undefined" || !crypto.getRandomValues) return null;
  const traceId = new Uint8Array(16);
  const spanId = new Uint8Array(8);
  crypto.getRandomValues(traceId);
  crypto.getRandomValues(spanId);
  return `00-${toHex(traceId)}-${toHex(spanId)}-01`;
};

const getTelemetryRunId = (): string | null => {
  try {
    return sessionStorage.getItem("ctxRunId");
  } catch {
    return null;
  }
};

const queueClientTelemetry = (event: ClientTelemetryMetric) => {
  if (typeof window === "undefined") return;
  if (clientTelemetryQueue.length >= CLIENT_TELEMETRY_MAX) return;
  clientTelemetryQueue.push(event);
  if (clientTelemetryTimer !== null) return;
  clientTelemetryTimer = window.setTimeout(() => {
    clientTelemetryTimer = null;
    flushClientTelemetry().catch(() => {});
  }, CLIENT_TELEMETRY_FLUSH_MS);
};

const queueSemanticTelemetry = (event: SemanticTelemetryEvent) => {
  if (typeof window === "undefined") return;
  if (event.delivery !== "local_only" && !semanticTelemetryRemoteEnabled) {
    return;
  }
  if (semanticTelemetryQueue.length >= CLIENT_TELEMETRY_MAX) {
    semanticTelemetryQueue.shift();
  }
  semanticTelemetryQueue.push(event);
  if (semanticTelemetryTimer !== null) return;
  semanticTelemetryTimer = window.setTimeout(() => {
    semanticTelemetryTimer = null;
    flushSemanticTelemetry().catch(() => {});
  }, CLIENT_TELEMETRY_FLUSH_MS);
};

const shouldRecordClientTelemetry = (path: string): boolean =>
  path.startsWith("/api/") && !path.startsWith("/api/telemetry");

const recordClientApiMetric = (
  path: string,
  method: string,
  status: number | null,
  ok: boolean,
  durationMs: number,
  runId: string | null,
) => {
  if (!shouldRecordClientTelemetry(path) || typeof window === "undefined") return;
  const endpoint = normalizePath(path);
  queueClientTelemetry({
    name: "client.api.duration_ms",
    kind: "histogram",
    unit: "ms",
    value: durationMs,
    run_id: runId,
    labels: {
      endpoint,
      method,
      status: status === null ? "error" : String(status),
      success: ok ? "true" : "false",
      source: "client",
    },
  });
};

const recordClientApiError = (path: string, method: string, durationMs: number, runId: string | null) => {
  if (!shouldRecordClientTelemetry(path) || typeof window === "undefined") return;
  const endpoint = normalizePath(path);
  queueClientTelemetry({
    name: "client.api.error_count",
    kind: "counter",
    unit: "count",
    value: 1,
    run_id: runId,
    labels: {
      endpoint,
      method,
      status: "error",
      success: "false",
      source: "client",
    },
  });
  recordClientApiMetric(path, method, null, false, durationMs, runId);
};

export const recordClientCounterMetric = (
  name: string,
  labels: Record<string, string> = {},
  value = 1,
): void => {
  recordClientMetric("counter", name, "count", value, labels);
};

export const recordClientHistogramMetric = (
  name: string,
  unit: string,
  value: number,
  labels: Record<string, string> = {},
): void => {
  recordClientMetric("histogram", name, unit, value, labels);
};

export const recordClientGaugeMetric = (
  name: string,
  unit: string,
  value: number,
  labels: Record<string, string> = {},
): void => {
  recordClientMetric("gauge", name, unit, value, labels);
};

const recordClientMetric = (
  kind: ClientTelemetryMetric["kind"],
  name: string,
  unit: string,
  value: number,
  labels: Record<string, string> = {},
) => {
  if (!name.trim() || typeof window === "undefined") return;
  queueClientTelemetry({
    name,
    kind,
    unit,
    value,
    run_id: getTelemetryRunId(),
    labels: {
      source: "client",
      ...labels,
    },
  });
};

export const recordSemanticTelemetryEvent = (event: SemanticTelemetryEvent): void => {
  if (!event.event_name.trim() || !event.origin_install_id.trim()) return;
  queueSemanticTelemetry(event);
};

export const setSemanticTelemetryRemoteEnabled = (enabled: boolean): void => {
  semanticTelemetryRemoteEnabled = enabled;
  if (enabled) return;
  for (let index = semanticTelemetryQueue.length - 1; index >= 0; index -= 1) {
    if (semanticTelemetryQueue[index]?.delivery !== "local_only") {
      semanticTelemetryQueue.splice(index, 1);
    }
  }
  if (!semanticTelemetryQueue.length && semanticTelemetryTimer !== null && typeof window !== "undefined") {
    window.clearTimeout(semanticTelemetryTimer);
    semanticTelemetryTimer = null;
  }
};

const flushClientTelemetry = async () => {
  if (!clientTelemetryQueue.length) return;
  const batch: ClientTelemetryBatch = { events: clientTelemetryQueue.splice(0) };
  await postTelemetryBatch(CLIENT_TELEMETRY_PATH, batch, "client_telemetry_flush");
};

const flushSemanticTelemetry = async () => {
  if (!semanticTelemetryQueue.length) return;
  const events = semanticTelemetryQueue
    .splice(0)
    .filter((event) => semanticTelemetryRemoteEnabled || event.delivery === "local_only");
  if (!events.length) return;
  const batch: SemanticTelemetryBatch = { events };
  await postTelemetryBatch(SEMANTIC_TELEMETRY_PATH, batch, "semantic_telemetry_flush");
};

const postTelemetryBatch = async (
  path: string,
  batch: ClientTelemetryBatch | SemanticTelemetryBatch,
  reason: "client_telemetry_flush" | "semantic_telemetry_flush",
) => {
  const token = authToken();
  try {
    if (isDesktopApp()) {
      await ensureDesktopDaemonConnection({
        connectLocalWhenMissing: false,
        reason,
      });
    }
    if (typeof fetch === "undefined") return;
    await fetch(getDaemonHttpUrl(path), {
      method: "POST",
      headers: {
        "content-type": "application/json",
        ...(token ? { authorization: `Bearer ${token}` } : {}),
      },
      body: JSON.stringify(batch),
      keepalive: true,
    });
  } catch {
    // Ignore telemetry upload failures.
  }
};

export type DaemonRawResponse = {
  status: number;
  body: string;
  content_type: string;
};

// For endpoints that need to handle non-2xx statuses without throwing (e.g. buffers update conflict 409).
export const daemonFetchRaw = async (path: string, init?: RequestInit): Promise<DaemonRawResponse> => {
  const method = init?.method ? String(init.method) : "GET";
  const traceparent = createTraceparent();
  const runId = getTelemetryRunId();

  const start =
    typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();

  if (isDesktopApp()) {
    await ensureDesktopDaemonConnection({
      connectLocalWhenMissing: true,
      reason: "daemon_fetch_raw_preflight",
    });
  }
  const token = authToken();

  try {
    const res = await fetch(getDaemonHttpUrl(path), {
      ...init,
      headers: buildDaemonRequestHeaders({
        headers: init?.headers,
        token,
        traceparent,
        runId,
      }),
    });
    const text = await res.text();
    const end = typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
    recordClientApiMetric(path, method, res.status, res.status < 500, end - start, runId);
    return {
      status: res.status,
      body: text,
      content_type: res.headers.get("content-type") ?? "",
    };
  } catch (err) {
    const end = typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
    recordClientApiError(path, method, end - start, runId);
    emitApiDiagnostic({
      path,
      method,
      code: "api.transport_error",
      severity: "error",
      message: normalizeDiagnosticErrorMessage(err, "Raw daemon fetch failed."),
    });
    throw err;
  }
};

export const idToString = (id: string | null | undefined): string => {
  if (id === null || id === undefined) return "";
  if (typeof id !== "string") {
    throw new Error("Expected id to be a string");
  }
  return id;
};
