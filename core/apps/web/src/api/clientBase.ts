import type { SemanticTelemetryEvent } from "@ctx/types";
import { isDesktopApp } from "../utils/desktop";
import { desktopDaemonRequest } from "../utils/desktop";
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
import {
  createTraceparent,
  getTelemetryRunId,
  recordClientApiError,
  recordClientApiMetric,
  recordClientCounterMetric,
  recordClientGaugeMetric,
  recordClientHistogramMetric,
  recordSemanticTelemetryEvent,
  setSemanticTelemetryRemoteEnabled,
} from "./clientBaseTelemetry";

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

const looksLikeHtml = (text: string): boolean => {
  const t = String(text || "").trimStart().toLowerCase();
  return t.startsWith("<!doctype html") || t.startsWith("<html");
};

const trimForError = (text: string): string => {
  const s = String(text || "").trim();
  if (s.length <= 800) return s;
  return `${s.slice(0, 800)}…`;
};

const toDesktopRequestHeaders = (
  headers: HeadersInit | undefined,
  traceparent: string | null,
  runId: string | null,
): [string, string][] => {
  const merged = buildDaemonRequestHeaders({
    headers,
    token: null,
    traceparent,
    runId,
  });
  return Object.entries(merged).filter(([key]) => key.toLowerCase() !== "authorization");
};

const parseDesktopJsonResponse = <T>(
  path: string,
  status: number,
  contentType: string,
  text: string,
): T => {
  if (status === 204) {
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

  if (!res.ok) {
    const text = await res.text();
    const contentType = res.headers.get("content-type") ?? "";

    if ((contentType.includes("text/html") || looksLikeHtml(text)) && path.startsWith("/api/")) {
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
  const traceparent = createTraceparent();
  const runId = getTelemetryRunId();
  const method = init?.method ? String(init.method) : "GET";
  const start = typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
  let res: DaemonRawResponse;
  try {
    res = await desktopDaemonRequest({
      method,
      path,
      headers: toDesktopRequestHeaders(init?.headers, traceparent, runId),
      body: typeof init?.body === "string" ? init.body : null,
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

  const contentType = String(res.content_type ?? "");
  const text = String(res.body ?? "");
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

  return parseDesktopJsonResponse<T>(path, res.status, contentType, text);
};

export const apiAny = async <T>(path: string, init?: RequestInit): Promise<T> => {
  if (isDesktopApp()) return desktopApi<T>(path, init);
  return api<T>(path, init);
};

export type DaemonRawResponse = {
  status: number;
  body: string;
  content_type: string;
};

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
    try {
      const res = await desktopDaemonRequest({
        method,
        path,
        headers: toDesktopRequestHeaders(init?.headers, traceparent, runId),
        body: typeof init?.body === "string" ? init.body : null,
      });
      const end = typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
      recordClientApiMetric(path, method, res.status, res.status < 500, end - start, runId);
      return {
        status: res.status,
        body: res.body,
        content_type: res.content_type ?? "",
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

export {
  recordClientCounterMetric,
  recordClientGaugeMetric,
  recordClientHistogramMetric,
  recordSemanticTelemetryEvent,
  setSemanticTelemetryRemoteEnabled,
};

export type { SemanticTelemetryEvent };
