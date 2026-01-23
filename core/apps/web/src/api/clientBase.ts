import type { ClientTelemetryBatch } from "@ctx/types";
import { desktopDaemonRequest, isDesktopApp } from "../utils/desktop";

export const authToken = (): string | null => {
  try {
    return sessionStorage.getItem("ctxAuthToken");
  } catch {
    return null;
  }
};

export const getDaemonBaseUrl = (): string | null => {
  try {
    return sessionStorage.getItem("contextDaemonBaseUrl") || localStorage.getItem("contextDaemonBaseUrl");
  } catch {
    return null;
  }
};

const isLoopbackHost = (host: string): boolean => {
  const normalized = host.replace(/^\[|\]$/g, "").toLowerCase();
  return normalized === "localhost" || normalized === "::1" || normalized.startsWith("127.");
};

export const resolveDaemonBaseUrl = (): string | null => {
  const base = getDaemonBaseUrl();
  if (!base) return null;
  if (typeof window === "undefined") return base;
  try {
    const baseUrl = new URL(base, window.location.origin);
    const originUrl = new URL(window.location.origin);
    if (isLoopbackHost(baseUrl.hostname) && !isLoopbackHost(originUrl.hostname)) {
      return window.location.origin;
    }
  } catch {
    return base;
  }
  return base;
};

const normalizeWsBaseUrl = (base: string): string => {
  const trimmed = base.replace(/\/+$/, "");
  if (trimmed.startsWith("ws://") || trimmed.startsWith("wss://")) return trimmed;
  if (trimmed.startsWith("https://")) return trimmed.replace(/^https:\/\//, "wss://");
  if (trimmed.startsWith("http://")) return trimmed.replace(/^http:\/\//, "ws://");
  return trimmed;
};

export const resolveDaemonWsBaseUrl = (): string => {
  const base = resolveDaemonBaseUrl();
  if (base) return normalizeWsBaseUrl(base);
  if (typeof window === "undefined") return "";
  return `${window.location.protocol === "https:" ? "wss" : "ws"}://${window.location.host}`;
};

export const setDaemonBaseUrl = (baseUrl: string | null, persist?: boolean) => {
  try {
    if (!baseUrl) {
      sessionStorage.removeItem("contextDaemonBaseUrl");
      if (persist) localStorage.removeItem("contextDaemonBaseUrl");
      return;
    }
    sessionStorage.setItem("contextDaemonBaseUrl", baseUrl);
    if (persist) localStorage.setItem("contextDaemonBaseUrl", baseUrl);
  } catch {
    // ignore
  }
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
      Object.assign(extraHeaders, init.headers as any);
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
      throw new Error(
        `The daemon returned HTML for ${path} (${res.status}). Restart/update the daemon (and ensure Vite is proxying /api to it).`,
      );
    }

    const lowered = String(text || "").toLowerCase();
    if (
      res.status >= 500 &&
      (lowered.includes("econnrefused") ||
        lowered.includes("proxy error") ||
        lowered.includes("connect econnrefused") ||
        lowered.includes("socket hang up"))
    ) {
      throw new Error(
        "Cannot reach the ctx daemon via /api. If you're running the web dev server, start the daemon (default http://127.0.0.1:4399) or set CTX_DAEMON_URL before `pnpm dev`.",
      );
    }
    try {
      const parsed = text ? JSON.parse(text) : null;
      const msg = parsed?.error ?? parsed?.message;
      if (typeof msg === "string" && msg.length > 0) {
        throw new Error(msg);
      }
    } catch {
      // ignore
    }
    throw new Error(trimForError(text) || `${res.status} ${res.statusText}`);
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
      Object.assign(extraHeaders, init.headers as any);
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
      throw new Error(
        `The daemon returned HTML for ${path} (${resp.status}). Restart/update the daemon.`,
      );
    }
    const lowered = String(text || "").toLowerCase();
    if (
      resp.status >= 500 &&
      (lowered.includes("econnrefused") ||
        lowered.includes("proxy error") ||
        lowered.includes("connect econnrefused") ||
        lowered.includes("socket hang up"))
    ) {
      throw new Error("Cannot reach the ctx daemon. Connect to a host from the launcher first.");
    }
    try {
      const parsed = text ? JSON.parse(text) : null;
      const msg = parsed?.error ?? parsed?.message;
      if (typeof msg === "string" && msg.length > 0) {
        throw new Error(msg);
      }
    } catch {
      // ignore
    }
    throw new Error(trimForError(text) || `${resp.status}`);
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
      Object.assign(extraHeaders, init.headers as any);
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
    throw err;
  }
};

export const idToString = (id: any): string =>
  typeof id === "string" ? id : id?.["0"];
