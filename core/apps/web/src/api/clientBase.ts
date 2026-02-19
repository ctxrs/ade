import type { ClientTelemetryBatch } from "@ctx/types";
import { desktopDaemonRequest, desktopGetConnection, isDesktopApp, type DesktopConnectionInfo } from "../utils/desktop";
import { emitUiDiagnostic, normalizeDiagnosticErrorMessage } from "../state/diagnosticsChannel";
import {
  applyDesktopDaemonConnection,
  bootstrapDaemonConnectionFromRuntime,
  clearDaemonConnection,
  getDaemonConnection,
  normalizeDaemonBaseUrl,
  setDaemonConnection,
  subscribeDaemonConnection,
} from "./daemonConnection";

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
  info: DesktopConnectionInfo | null;
  synced: boolean;
  error: string | null;
};

type DesktopDaemonConnectionSyncOptions = {
  force?: boolean;
  probeHealth?: boolean;
  reason?: string;
};

const DESKTOP_DAEMON_SYNC_THROTTLE_MS = 1000;
let desktopSyncInFlight: Promise<DesktopDaemonConnectionSyncResult> | null = null;
let desktopLastSyncAtMs = 0;

const makeDesktopSyncResult = (
  info: DesktopConnectionInfo | null,
  error: string | null,
): DesktopDaemonConnectionSyncResult => ({
  config: getDaemonClientConfig(),
  info,
  synced: Boolean(info),
  error,
});

// Desktop invariant: worker and main-thread HTTP clients should share the same daemon connection state.
// Bridge-backed requests can succeed before JS state is hydrated, so we opportunistically reconcile here.
export const syncDesktopDaemonConnectionFromBridge = async (
  opts?: DesktopDaemonConnectionSyncOptions,
): Promise<DesktopDaemonConnectionSyncResult> => {
  if (!isDesktopApp()) return makeDesktopSyncResult(null, null);
  const now = Date.now();
  const current = getDaemonConnection();
  if (
    !opts?.force
    && current.baseUrl
    && now - desktopLastSyncAtMs < DESKTOP_DAEMON_SYNC_THROTTLE_MS
  ) {
    return makeDesktopSyncResult(null, null);
  }
  if (desktopSyncInFlight) return desktopSyncInFlight;
  const run = (async (): Promise<DesktopDaemonConnectionSyncResult> => {
    let info: DesktopConnectionInfo | null = null;
    let error: string | null = null;
    try {
      info = await desktopGetConnection();
      const shouldProbeHealth = (opts?.probeHealth ?? true) && (!info.base_url || info.kind === "none");
      if (shouldProbeHealth) {
        try {
          await desktopDaemonRequest({
            method: "GET",
            path: "/api/health",
            body: null,
            headers: [["content-type", "application/json"]],
          });
        } catch {
          // ignore probe failures; caller will still receive the refreshed bridge state
        }
        try {
          info = await desktopGetConnection();
        } catch {
          // ignore and use the earlier connection snapshot
        }
      }
      applyDesktopDaemonConnection(info);
    } catch (err) {
      error = normalizeDiagnosticErrorMessage(err, "Desktop daemon connection sync failed.");
      if (opts?.reason) {
        emitUiDiagnostic({
          source: "api",
          code: "api.desktop_connection_sync_failed",
          severity: "warning",
          message: `Desktop daemon connection sync failed during ${opts.reason}.`,
          context: { reason: opts.reason, error },
        });
      }
    } finally {
      desktopLastSyncAtMs = Date.now();
    }
    return makeDesktopSyncResult(info, error);
  })();
  desktopSyncInFlight = run;
  try {
    return await run;
  } finally {
    if (desktopSyncInFlight === run) {
      desktopSyncInFlight = null;
    }
  }
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
  const extraHeaders: Record<string, string> = {};
  if (init?.headers) {
    if (init.headers instanceof Headers) {
      init.headers.forEach((value, key) => {
        extraHeaders[key] = value;
      });
    } else if (Array.isArray(init.headers)) {
      for (const [key, value] of init.headers) {
        extraHeaders[key] = value;
      }
    } else {
      Object.assign(extraHeaders, init.headers as Record<string, string>);
    }
  }
  const traceparent = createTraceparent();
  if (traceparent && !extraHeaders.traceparent) {
    extraHeaders.traceparent = traceparent;
  }
  const runId = getTelemetryRunId();
  if (runId && !extraHeaders["x-ctx-run-id"]) {
    extraHeaders["x-ctx-run-id"] = runId;
  }
  const method = init?.method ? String(init.method) : "GET";
  const start = typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
  let res: Response;
  try {
    res = await fetch(path, {
      headers: {
        "content-type": "application/json",
        ...(token ? { authorization: `Bearer ${token}` } : {}),
        ...extraHeaders,
      },
      ...init,
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
  if (!getDaemonConnection().baseUrl) {
    await syncDesktopDaemonConnectionFromBridge({
      force: true,
      probeHealth: true,
      reason: "desktop_api_preflight",
    });
  } else {
    void syncDesktopDaemonConnectionFromBridge({
      force: false,
      probeHealth: false,
      reason: "desktop_api_background",
    }).catch(() => {});
  }

  const extraHeaders: Record<string, string> = {};
  if (init?.headers) {
    if (init.headers instanceof Headers) {
      init.headers.forEach((value, key) => {
        extraHeaders[key] = value;
      });
    } else if (Array.isArray(init.headers)) {
      for (const [key, value] of init.headers) {
        extraHeaders[key] = value;
      }
    } else {
      Object.assign(extraHeaders, init.headers as Record<string, string>);
    }
  }
  const traceparent = createTraceparent();
  if (traceparent && !extraHeaders.traceparent) {
    extraHeaders.traceparent = traceparent;
  }
  const runId = getTelemetryRunId();
  if (runId && !extraHeaders["x-ctx-run-id"]) {
    extraHeaders["x-ctx-run-id"] = runId;
  }

  const method = init?.method ? String(init.method) : "GET";
  const body =
    init?.body === undefined || init?.body === null
      ? null
      : typeof init.body === "string"
        ? init.body
        : String(init.body);

  const start = typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
  let resp;
  try {
    resp = await desktopDaemonRequest({
      method,
      path,
      body,
      headers: Object.entries({
        "content-type": "application/json",
        ...extraHeaders,
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
  recordClientApiMetric(path, method, resp.status, resp.status < 500, end - start, runId);

  const contentType = String(resp.content_type ?? "");
  const text = String(resp.body ?? "");

  const looksLikeHtml = (t: string): boolean => {
    const s = String(t || "").trimStart().toLowerCase();
    return s.startsWith("<!doctype html") || s.startsWith("<html");
  };

  const trimForError = (t: string): string => {
    const s = String(t || "").trim();
    if (s.length <= 800) return s;
    return `${s.slice(0, 800)}…`;
  };

  const ok = resp.status >= 200 && resp.status < 300;
  if (!ok) {
    if ((contentType.includes("text/html") || looksLikeHtml(text)) && path.startsWith("/api/")) {
      const message = `The daemon returned HTML for ${path} (${resp.status}). Restart/update the daemon.`;
      emitApiDiagnostic({
        path,
        method,
        status: resp.status,
        code: "api.http_error",
        severity: "error",
        message,
      });
      throw new Error(message);
    }
    const lowered = String(text || "").toLowerCase();
    if (
      resp.status >= 500 &&
      (lowered.includes("econnrefused") ||
        lowered.includes("proxy error") ||
        lowered.includes("connect econnrefused") ||
        lowered.includes("socket hang up"))
    ) {
      const message = "Cannot reach the ctx daemon. Connect to a host from the launcher first.";
      emitApiDiagnostic({
        path,
        method,
        status: resp.status,
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
    const message = parsedMessage ?? (trimForError(text) || `${resp.status}`);
    emitApiDiagnostic({
      path,
      method,
      status: resp.status,
      code: "api.http_error",
      severity: resp.status >= 500 ? "error" : "warning",
      message,
    });
    throw new Error(message);
  }

  if (resp.status === 204) {
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
const CLIENT_TELEMETRY_FLUSH_MS = 1000;
const CLIENT_TELEMETRY_MAX = 200;
let clientTelemetryTimer: number | null = null;
const clientTelemetryQueue: ClientTelemetryMetric[] = [];

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
  if (!name.trim() || typeof window === "undefined") return;
  queueClientTelemetry({
    name,
    kind: "counter",
    unit: "count",
    value,
    run_id: getTelemetryRunId(),
    labels: {
      source: "client",
      ...labels,
    },
  });
};

const flushClientTelemetry = async () => {
  if (!clientTelemetryQueue.length) return;
  const batch: ClientTelemetryBatch = { events: clientTelemetryQueue.splice(0) };
  const token = authToken();
  try {
    if (isDesktopApp()) {
      await desktopDaemonRequest({
        method: "POST",
        path: CLIENT_TELEMETRY_PATH,
        body: JSON.stringify(batch),
        headers: Object.entries({
          "content-type": "application/json",
          ...(token ? { authorization: `Bearer ${token}` } : {}),
        }),
      });
      return;
    }
    if (typeof fetch === "undefined") return;
    await fetch(CLIENT_TELEMETRY_PATH, {
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
  const token = authToken();
  const extraHeaders: Record<string, string> = {};
  if (init?.headers) {
    if (init.headers instanceof Headers) {
      init.headers.forEach((value, key) => {
        extraHeaders[key] = value;
      });
    } else if (Array.isArray(init.headers)) {
      for (const [key, value] of init.headers) {
        extraHeaders[key] = value;
      }
    } else {
      Object.assign(extraHeaders, init.headers as Record<string, string>);
    }
  }

  const method = init?.method ? String(init.method) : "GET";
  const body =
    init?.body === undefined || init?.body === null
      ? null
      : typeof init.body === "string"
        ? init.body
        : String(init.body);

  const traceparent = createTraceparent();
  if (traceparent && !extraHeaders.traceparent) {
    extraHeaders.traceparent = traceparent;
  }
  const runId = getTelemetryRunId();
  if (runId && !extraHeaders["x-ctx-run-id"]) {
    extraHeaders["x-ctx-run-id"] = runId;
  }

  const start =
    typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();

  if (isDesktopApp()) {
    if (!getDaemonConnection().baseUrl) {
      await syncDesktopDaemonConnectionFromBridge({
        force: true,
        probeHealth: true,
        reason: "daemon_fetch_raw_preflight",
      });
    } else {
      void syncDesktopDaemonConnectionFromBridge({
        force: false,
        probeHealth: false,
        reason: "daemon_fetch_raw_background",
      }).catch(() => {});
    }

    const resp = await desktopDaemonRequest({
      method,
      path,
      body,
      headers: Object.entries({
        "content-type": "application/json",
        ...extraHeaders,
      }),
    });
    const end = typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
    recordClientApiMetric(path, method, resp.status, resp.status < 500, end - start, runId);
    return {
      status: resp.status,
      body: String(resp.body ?? ""),
      content_type: String(resp.content_type ?? ""),
    };
  }

  try {
    const res = await fetch(path, {
      headers: {
        ...(token ? { authorization: `Bearer ${token}` } : {}),
        ...extraHeaders,
      },
      ...init,
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
