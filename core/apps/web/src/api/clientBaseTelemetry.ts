import type { ClientTelemetryBatch, SemanticTelemetryBatch, SemanticTelemetryEvent } from "@ctx/types";
import { isDesktopApp } from "../utils/desktop";
import { ensureDesktopDaemonConnection } from "./desktopDaemonConnection";
import { getDaemonConnection, getDaemonHttpUrl } from "./daemonConnection";

const CLIENT_TELEMETRY_PATH = "/api/telemetry/client";
const SEMANTIC_TELEMETRY_PATH = "/api/telemetry/events";
const CLIENT_TELEMETRY_FLUSH_MS = 1000;
const CLIENT_TELEMETRY_MAX = 200;

type ClientTelemetryMetric = {
  name: string;
  kind: "histogram" | "counter" | "gauge";
  unit: string;
  value: number;
  labels?: Record<string, string>;
  run_id?: string | null;
};

let clientTelemetryTimer: number | null = null;
const clientTelemetryQueue: ClientTelemetryMetric[] = [];
let semanticTelemetryTimer: number | null = null;
const semanticTelemetryQueue: SemanticTelemetryEvent[] = [];
let semanticTelemetryRemoteEnabled = true;

export const resetClientBaseTelemetryForTests = (): void => {
  if (typeof window !== "undefined") {
    if (clientTelemetryTimer !== null) {
      window.clearTimeout(clientTelemetryTimer);
    }
    if (semanticTelemetryTimer !== null) {
      window.clearTimeout(semanticTelemetryTimer);
    }
  }
  clientTelemetryTimer = null;
  clientTelemetryQueue.splice(0);
  semanticTelemetryTimer = null;
  semanticTelemetryQueue.splice(0);
  semanticTelemetryRemoteEnabled = true;
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

export const createTraceparent = (): string | null => {
  if (typeof crypto === "undefined" || !crypto.getRandomValues) return null;
  const traceId = new Uint8Array(16);
  const spanId = new Uint8Array(8);
  crypto.getRandomValues(traceId);
  crypto.getRandomValues(spanId);
  return `00-${toHex(traceId)}-${toHex(spanId)}-01`;
};

export const getTelemetryRunId = (): string | null => {
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

export const recordClientApiMetric = (
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

export const recordClientApiError = (path: string, method: string, durationMs: number, runId: string | null) => {
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
  try {
    if (isDesktopApp()) {
      await ensureDesktopDaemonConnection({
        force: true,
        connectLocalWhenMissing: false,
        reason,
      });
    }
    if (typeof fetch === "undefined") return;
    const token = getDaemonConnection().authToken;
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
