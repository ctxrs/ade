import type { ProfilerOnRenderCallback } from "react";
import { randomUuid } from "./randomUuid";

type WalMode = "off" | "light" | "heavy";

type WalEvent = {
  seq: number;
  ts_ms: number;
  kind: string;
  session_id: string;
  page_id: string;
  data?: Record<string, unknown>;
};

type WalRecorder = {
  mode: WalMode;
  sessionId: string;
  pageId: string;
  endpoint: string;
  record: (kind: string, data?: Record<string, unknown>, opts?: { level?: "light" | "heavy" }) => void;
  dump: (opts?: { limit?: number }) => WalEvent[];
  flush: (reason?: "interval" | "manual" | "unload") => void;
  setMode: (mode: WalMode) => void;
  getStatus: () => {
    mode: WalMode;
    sessionId: string;
    pageId: string;
    ringSize: number;
    queueSize: number;
    dropped: number;
    endpoint: string;
    lastFlushMs: number | null;
  };
  onRender?: ProfilerOnRenderCallback;
};

const MAX_RING = 5000;
const MAX_QUEUE = 2000;
const FLUSH_MS = 2000;
const MAX_STRING_LIGHT = 500;
const MAX_STRING_HEAVY = 2000;
const WS_IDLE_MS = 30000;
const WS_SAMPLE_MS = 2000;
const WAL_ENDPOINT_DEFAULT = "/__ctx_wal__";

const REDACT_HEADERS = new Set([
  "authorization",
  "cookie",
  "set-cookie",
  "x-api-key",
  "x-supabase-key",
  "x-ctx-auth",
  "x-ctx-token",
]);

const globalAny = globalThis as unknown as {
  __CTX_WAL__?: WalRecorder;
  __CTX_WAL_HOOKS__?: boolean;
};

const nowMs = (): number => {
  if (typeof performance !== "undefined" && typeof performance.now === "function") {
    return (performance.timeOrigin ?? Date.now()) + performance.now();
  }
  return Date.now();
};

const clampString = (value: string, maxLen: number): string => {
  if (value.length <= maxLen) return value;
  return `${value.slice(0, maxLen)}...`;
};

const normalizeUrl = (raw: string): string => {
  try {
    const base =
      typeof window !== "undefined" && window.location ? window.location.origin : "http://localhost";
    const url = new URL(raw, base);
    const cleanParams = new URLSearchParams();
    for (const key of url.searchParams.keys()) {
      cleanParams.append(key, "");
    }
    const search = cleanParams.toString();
    return `${url.origin}${url.pathname}${search ? `?${search}` : ""}`;
  } catch {
    return raw;
  }
};

const sanitizeHeaders = (
  headers: Headers | Record<string, string> | Array<[string, string]> | string | null | undefined,
): Record<string, string> | undefined => {
  if (!headers) return undefined;
  let entries: Array<[string, string]> = [];
  if (typeof headers === "string") {
    entries = headers
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean)
      .map((line) => {
        const idx = line.indexOf(":");
        if (idx === -1) return [line, ""];
        return [line.slice(0, idx), line.slice(idx + 1).trim()];
      });
  } else if (headers instanceof Headers) {
    entries = Array.from(headers.entries());
  } else if (Array.isArray(headers)) {
    entries = headers;
  } else {
    entries = Object.entries(headers);
  }
  const out: Record<string, string> = {};
  for (const [key, value] of entries) {
    const lower = key.toLowerCase();
    const safeValue = REDACT_HEADERS.has(lower) ? "<redacted>" : clampString(String(value ?? ""), MAX_STRING_LIGHT);
    out[key] = safeValue;
  }
  return out;
};

const serializeConsoleArg = (value: unknown, mode: WalMode): unknown => {
  const maxLen = mode === "heavy" ? MAX_STRING_HEAVY : MAX_STRING_LIGHT;
  if (value instanceof Error) {
    return {
      type: "error",
      name: value.name,
      message: clampString(value.message ?? "", maxLen),
      stack: value.stack ? clampString(value.stack, maxLen) : undefined,
    };
  }
  if (typeof value === "string") return clampString(value, maxLen);
  if (typeof value === "number" || typeof value === "boolean" || value === null) return value;
  if (typeof value === "undefined") return "undefined";
  try {
    const json = JSON.stringify(value);
    if (json.length <= maxLen) return JSON.parse(json);
    return clampString(json, maxLen);
  } catch {
    return Object.prototype.toString.call(value);
  }
};

const shouldDisable = (): boolean => {
  if (typeof window === "undefined") return true;
  if (import.meta.env.MODE === "test") return true;
  return false;
};

const resolveMode = (): WalMode => {
  if (shouldDisable()) return "off";
  const query = typeof window !== "undefined" ? new URLSearchParams(window.location.search) : null;
  const queryMode = query?.get("wal")?.toLowerCase();
  if (queryMode === "0" || queryMode === "off" || queryMode === "false") return "off";
  if (queryMode === "heavy") return "heavy";
  if (queryMode === "light" || queryMode === "1" || queryMode === "true") return "light";

  const envMode = String(import.meta.env.VITE_CTX_WAL_MODE ?? "").toLowerCase();
  if (envMode === "off" || envMode === "0" || envMode === "false") return "off";
  if (envMode === "heavy") return "heavy";
  if (envMode === "light" || envMode === "1" || envMode === "true") return "light";

  return import.meta.env.DEV ? "light" : "off";
};

const resolveEndpoint = (): string => {
  const raw = String(import.meta.env.VITE_CTX_WAL_ENDPOINT ?? "").trim();
  return raw.length > 0 ? raw : WAL_ENDPOINT_DEFAULT;
};

const getSessionId = (): string => {
  try {
    const existing = sessionStorage.getItem("ctxWalSessionId");
    if (existing) return existing;
    const created = randomUuid();
    sessionStorage.setItem("ctxWalSessionId", created);
    return created;
  } catch {
    return randomUuid();
  }
};

const parseContentLength = (value: string | null): number | undefined => {
  if (!value) return undefined;
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : undefined;
};

const installHooks = (recorder: WalRecorder, getMode: () => WalMode) => {
  if (globalAny.__CTX_WAL_HOOKS__) return;
  globalAny.__CTX_WAL_HOOKS__ = true;

  if (typeof window !== "undefined" && window.fetch) {
    const originalFetch = window.fetch.bind(window);
    window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
      const rawUrl = typeof input === "string" ? input : "url" in input ? input.url : String(input);
      const method =
        init?.method ??
        (typeof Request !== "undefined" && input instanceof Request ? input.method : "GET");
      const normalizedUrl = normalizeUrl(rawUrl);
      const skip =
        normalizedUrl.includes(WAL_ENDPOINT_DEFAULT) ||
        Boolean((init?.headers as Record<string, string> | undefined)?.["x-ctx-wal"]);
      if (skip) return originalFetch(input, init);

      const start = nowMs();
      let res: Response | null = null;
      let error: unknown = null;
      try {
        res = await originalFetch(input, init);
        return res;
      } catch (err) {
        error = err;
        throw err;
      } finally {
        const durationMs = nowMs() - start;
        const mode = getMode();
        const requestBytes =
          typeof init?.body === "string"
            ? init.body.length
            : init?.body instanceof ArrayBuffer
              ? init.body.byteLength
              : init?.body instanceof Blob
                ? init.body.size
                : undefined;
        const requestHeaders = mode === "heavy" ? sanitizeHeaders(init?.headers ?? undefined) : undefined;
        recorder.record(
          "fetch",
          {
            url: normalizedUrl,
            method: String(method ?? "GET"),
            status: res?.status ?? null,
            ok: res?.ok ?? false,
            duration_ms: Math.round(durationMs),
            request_bytes: requestBytes,
            response_bytes: parseContentLength(res?.headers.get("content-length") ?? null),
            response_type: res?.type ?? null,
            response_content_type: res?.headers.get("content-type") ?? null,
            error: error instanceof Error ? { name: error.name, message: error.message } : undefined,
            request_headers: requestHeaders,
            response_headers: mode === "heavy" ? sanitizeHeaders(res?.headers ?? undefined) : undefined,
          },
          { level: "light" },
        );
      }
    };
  }

  if (typeof XMLHttpRequest !== "undefined") {
    const originalOpen = XMLHttpRequest.prototype.open;
    const originalSend = XMLHttpRequest.prototype.send;
    XMLHttpRequest.prototype.open = function (method: string, url: string, ...rest: any[]) {
      (this as any).__ctxWal = { method, url, start: 0, request_bytes: undefined };
      return originalOpen.call(this, method, url, ...rest);
    };
    XMLHttpRequest.prototype.send = function (body?: Document | BodyInit | null) {
      const meta = (this as any).__ctxWal;
      if (meta) {
        meta.start = nowMs();
        if (typeof body === "string") meta.request_bytes = body.length;
        if (body instanceof ArrayBuffer) meta.request_bytes = body.byteLength;
        if (body instanceof Blob) meta.request_bytes = body.size;
        const onLoadEnd = () => {
          this.removeEventListener("loadend", onLoadEnd);
          const durationMs = nowMs() - meta.start;
          const mode = getMode();
          const normalizedUrl = normalizeUrl(meta.url ?? "");
          if (!normalizedUrl.includes(WAL_ENDPOINT_DEFAULT)) {
            const responseHeaders = mode === "heavy" ? sanitizeHeaders(this.getAllResponseHeaders()) : undefined;
            recorder.record(
              "xhr",
              {
                url: normalizedUrl,
                method: String(meta.method ?? "GET"),
                status: typeof this.status === "number" ? this.status : null,
                ok: typeof this.status === "number" ? this.status >= 200 && this.status < 400 : false,
                duration_ms: Math.round(durationMs),
                request_bytes: meta.request_bytes,
                response_bytes: parseContentLength(this.getResponseHeader("content-length")),
                response_type: this.responseType ?? null,
                response_content_type: this.getResponseHeader("content-type"),
                response_headers: responseHeaders,
              },
              { level: "light" },
            );
          }
        };
        this.addEventListener("loadend", onLoadEnd);
      }
      return originalSend.call(this, body as any);
    };
  }

  if (typeof WebSocket !== "undefined") {
    const OriginalWebSocket = WebSocket;
    const socketState = new Map<
      string,
      { lastMessageMs: number; lastIdleMs: number; lastSampleMs: number; messageCount: number; bytes: number }
    >();
    class WrappedWebSocket extends OriginalWebSocket {
      __ctxWalId: string;
      constructor(url: string | URL, protocols?: string | string[]) {
        super(url, protocols as any);
        const id = randomUuid();
        this.__ctxWalId = id;
        const normalizedUrl = normalizeUrl(String(url));
        recorder.record("ws:open", { id, url: normalizedUrl });
        const now = nowMs();
        socketState.set(id, {
          lastMessageMs: now,
          lastIdleMs: 0,
          lastSampleMs: now,
          messageCount: 0,
          bytes: 0,
        });
        this.addEventListener("message", (event) => {
          const entry = socketState.get(id);
          const now = nowMs();
          if (entry) {
            entry.lastMessageMs = now;
            entry.messageCount += 1;
          }
          const mode = getMode();
          const size =
            typeof event.data === "string"
              ? event.data.length
              : event.data instanceof ArrayBuffer
                ? event.data.byteLength
                : event.data instanceof Blob
                  ? event.data.size
                  : undefined;
          if (entry && typeof size === "number") {
            entry.bytes += size;
          }
          if (mode === "heavy") {
            const preview =
              typeof event.data === "string" ? clampString(event.data, MAX_STRING_LIGHT) : undefined;
            recorder.record(
              "ws:message",
              {
                id,
                url: normalizedUrl,
                size,
                preview,
              },
              { level: "heavy" },
            );
          }
        });
        this.addEventListener("close", (event) => {
          socketState.delete(id);
          recorder.record("ws:close", {
            id,
            url: normalizedUrl,
            code: event.code,
            reason: event.reason,
            was_clean: event.wasClean,
          });
        });
        this.addEventListener("error", () => {
          recorder.record("ws:error", { id, url: normalizedUrl });
        });
      }
    }
    Object.assign(WrappedWebSocket, OriginalWebSocket);
    (window as unknown as { WebSocket: typeof WebSocket }).WebSocket = WrappedWebSocket;

    window.setInterval(() => {
      const now = nowMs();
      const mode = getMode();
      for (const [id, entry] of socketState.entries()) {
        if (mode !== "heavy" && entry.messageCount > 0 && now - entry.lastSampleMs >= WS_SAMPLE_MS) {
          recorder.record("ws:traffic", {
            id,
            count: entry.messageCount,
            bytes: entry.bytes,
            window_ms: Math.round(now - entry.lastSampleMs),
            last_message_ms: Math.round(entry.lastMessageMs),
          });
          entry.messageCount = 0;
          entry.bytes = 0;
          entry.lastSampleMs = now;
        }
        if (now - entry.lastMessageMs >= WS_IDLE_MS && now - entry.lastIdleMs >= WS_IDLE_MS) {
          entry.lastIdleMs = now;
          recorder.record("ws:idle", { id, idle_ms: Math.round(now - entry.lastMessageMs) });
        }
      }
    }, WS_SAMPLE_MS);
  }

  if (typeof window !== "undefined" && window.console) {
    const levels = ["log", "info", "warn", "error", "debug"] as const;
    for (const level of levels) {
      const original = window.console[level];
      if (typeof original !== "function") continue;
      window.console[level] = (...args: unknown[]) => {
        const mode = getMode();
        recorder.record("console", {
          level,
          args: args.map((arg) => serializeConsoleArg(arg, mode)),
        });
        return original.apply(window.console, args as any);
      };
    }
  }

  if (typeof window !== "undefined") {
    window.addEventListener("error", (event) => {
      const target = event.target as HTMLElement | null;
      const targetTag = target?.tagName?.toLowerCase();
      const targetUrl =
        target instanceof HTMLImageElement
          ? target.currentSrc || target.src
          : target instanceof HTMLScriptElement
            ? target.src
            : target instanceof HTMLLinkElement
              ? target.href
              : undefined;
      if (targetUrl) {
        recorder.record("resource:error", { tag: targetTag, url: normalizeUrl(targetUrl) });
        return;
      }
      const error = event.error as Error | undefined;
      recorder.record("error", {
        message: event.message,
        filename: event.filename,
        lineno: event.lineno,
        colno: event.colno,
        stack: error?.stack ? clampString(error.stack, MAX_STRING_LIGHT) : undefined,
      });
    });

    window.addEventListener("unhandledrejection", (event) => {
      const reason = event.reason;
      recorder.record("unhandledrejection", {
        reason: serializeConsoleArg(reason, getMode()),
      });
    });

    window.addEventListener("online", () => recorder.record("navigator:online"));
    window.addEventListener("offline", () => recorder.record("navigator:offline"));
    window.addEventListener("focus", () => recorder.record("window:focus"));
    window.addEventListener("blur", () => recorder.record("window:blur"));
    window.addEventListener("visibilitychange", () => {
      recorder.record("document:visibility", { state: document.visibilityState });
      if (document.visibilityState === "hidden") recorder.flush("unload");
    });
    window.addEventListener("pagehide", () => recorder.flush("unload"));
  }
};

const initPerformanceObservers = (recorder: WalRecorder, getMode: () => WalMode) => {
  if (typeof performance === "undefined" || typeof PerformanceObserver === "undefined") return;
  const supported = (PerformanceObserver as any).supportedEntryTypes as string[] | undefined;
  const supports = new Set(supported ?? []);
  const timeOrigin = performance.timeOrigin ?? Date.now();

  if (supports.has("navigation")) {
    const nav = performance.getEntriesByType("navigation")[0] as PerformanceNavigationTiming | undefined;
    if (nav) {
      recorder.record("perf:navigation", {
        type: nav.type,
        dom_interactive: Math.round(nav.domInteractive),
        dom_content_loaded: Math.round(nav.domContentLoadedEventEnd),
        load_event_end: Math.round(nav.loadEventEnd),
        redirect_count: nav.redirectCount,
        transfer_size: nav.transferSize,
      });
    }
  }

  if (supports.has("paint")) {
    const paints = performance.getEntriesByType("paint") as PerformanceEntry[];
    for (const entry of paints) {
      recorder.record("perf:paint", {
        name: entry.name,
        start_ms: Math.round(timeOrigin + entry.startTime),
      });
    }
  }

  if (supports.has("longtask")) {
    const observer = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        recorder.record("perf:longtask", {
          start_ms: Math.round(timeOrigin + entry.startTime),
          duration_ms: Math.round(entry.duration ?? 0),
        });
      }
    });
    try {
      observer.observe({ entryTypes: ["longtask"] });
    } catch {
      // ignore
    }
  }

  let clsTotal = 0;
  let lcpEntry: PerformanceEntry | null = null;

  if (supports.has("layout-shift")) {
    const observer = new PerformanceObserver((list) => {
      for (const entry of list.getEntries() as any[]) {
        if (!entry) continue;
        const value = typeof entry.value === "number" ? entry.value : 0;
        const hadRecentInput = Boolean(entry.hadRecentInput);
        if (!hadRecentInput) clsTotal += value;
        recorder.record("perf:layout-shift", {
          value,
          had_recent_input: hadRecentInput,
          cls_total: Number(clsTotal.toFixed(4)),
        });
      }
    });
    try {
      observer.observe({ entryTypes: ["layout-shift"] });
    } catch {
      // ignore
    }
  }

  if (supports.has("largest-contentful-paint")) {
    const observer = new PerformanceObserver((list) => {
      const entries = list.getEntries();
      if (!entries.length) return;
      lcpEntry = entries[entries.length - 1];
      const entry = lcpEntry as any;
      const mode = getMode();
      recorder.record(
        "perf:lcp",
        {
          start_ms: Math.round(timeOrigin + entry.startTime),
          size: entry.size,
          element: mode === "heavy" && entry.element ? entry.element.tagName : undefined,
          url: mode === "heavy" ? entry.url : undefined,
        },
        { level: mode === "heavy" ? "heavy" : "light" },
      );
    });
    try {
      observer.observe({ type: "largest-contentful-paint", buffered: true } as any);
    } catch {
      // ignore
    }
  }

  if (supports.has("first-input")) {
    const observer = new PerformanceObserver((list) => {
      for (const entry of list.getEntries() as any[]) {
        recorder.record("perf:first-input", {
          name: entry.name,
          start_ms: Math.round(timeOrigin + entry.startTime),
          duration_ms: Math.round(entry.duration ?? 0),
          processing_start: entry.processingStart ?? undefined,
        });
      }
    });
    try {
      observer.observe({ entryTypes: ["first-input"] });
    } catch {
      // ignore
    }
  }

  if (supports.has("event")) {
    const observer = new PerformanceObserver((list) => {
      for (const entry of list.getEntries() as any[]) {
        if (!entry) continue;
        recorder.record(
          "perf:event",
          {
            name: entry.name,
            start_ms: Math.round(timeOrigin + entry.startTime),
            duration_ms: Math.round(entry.duration ?? 0),
            interaction_id: entry.interactionId ?? undefined,
          },
          { level: "heavy" },
        );
      }
    });
    try {
      observer.observe({ type: "event", buffered: true, durationThreshold: 40 } as any);
    } catch {
      // ignore
    }
  }

  if (typeof window !== "undefined") {
    window.addEventListener("visibilitychange", () => {
      if (document.visibilityState !== "hidden") return;
      if (lcpEntry) {
        recorder.record("perf:lcp:final", {
          start_ms: Math.round(timeOrigin + lcpEntry.startTime),
        });
      }
      recorder.record("perf:cls:final", { cls_total: Number(clsTotal.toFixed(4)) });
    });
  }
};

export const initWalRecorder = (): WalRecorder | null => {
  if (shouldDisable()) return null;
  const mode = resolveMode();
  const existing = globalAny.__CTX_WAL__;
  if (existing) {
    existing.setMode(mode);
    return mode === "off" ? null : existing;
  }
  if (mode === "off") return null;

  const sessionId = getSessionId();
  const pageId = randomUuid();
  const endpoint = resolveEndpoint();

  let seq = 0;
  let dropped = 0;
  let lastFlushMs: number | null = null;
  const ring: WalEvent[] = [];
  const queue: WalEvent[] = [];
  let modeState: WalMode = mode;
  let flushTimer: number | null = null;

  const record = (kind: string, data?: Record<string, unknown>, opts?: { level?: "light" | "heavy" }) => {
    const level = opts?.level ?? "light";
    if (modeState === "off") return;
    if (level === "heavy" && modeState !== "heavy") return;
    const event: WalEvent = {
      seq: (seq += 1),
      ts_ms: nowMs(),
      kind,
      session_id: sessionId,
      page_id: pageId,
      data,
    };
    if (ring.length >= MAX_RING) ring.shift();
    ring.push(event);
    if (queue.length >= MAX_QUEUE) {
      queue.shift();
      dropped += 1;
    }
    queue.push(event);
    if (flushTimer === null && typeof window !== "undefined") {
      flushTimer = window.setTimeout(() => {
        flushTimer = null;
        flush("interval");
      }, FLUSH_MS);
    }
  };

  const buildEndpoint = () => {
    const base = endpoint;
    const separator = base.includes("?") ? "&" : "?";
    return `${base}${separator}session=${encodeURIComponent(sessionId)}`;
  };

  const flush = (reason: "interval" | "manual" | "unload" = "interval") => {
    if (queue.length === 0) return;
    const batch = queue.splice(0);
    const lines = batch
      .map((event) => {
        try {
          return JSON.stringify(event);
        } catch {
          return JSON.stringify({ kind: "wal:serialize_error", ts_ms: nowMs() });
        }
      })
      .join("\n");
    const payload = `${lines}\n`;
    const url = buildEndpoint();
    let sent = false;
    if (reason === "unload" && typeof navigator !== "undefined" && navigator.sendBeacon) {
      try {
        sent = navigator.sendBeacon(url, payload);
      } catch {
        sent = false;
      }
    }
    if (!sent && typeof fetch !== "undefined") {
      fetch(url, {
        method: "POST",
        headers: { "content-type": "text/plain", "x-ctx-wal": "1", "x-ctx-wal-session": sessionId },
        body: payload,
        keepalive: reason === "unload",
      }).catch(() => {
        for (const item of batch) {
          if (queue.length >= MAX_QUEUE) {
            queue.shift();
            dropped += 1;
          }
          queue.push(item);
        }
      });
    }
    lastFlushMs = nowMs();
  };

  const setMode = (next: WalMode) => {
    if (modeState === next) return;
    modeState = next;
    record("wal:mode", { mode: modeState });
  };

  const getStatus = () => ({
    mode: modeState,
    sessionId,
    pageId,
    ringSize: ring.length,
    queueSize: queue.length,
    dropped,
    endpoint,
    lastFlushMs,
  });

  const recorder: WalRecorder = {
    mode: modeState,
    sessionId,
    pageId,
    endpoint,
    record,
    dump: ({ limit } = {}) => {
      if (!limit || limit >= ring.length) return ring.slice();
      return ring.slice(Math.max(0, ring.length - limit));
    },
    flush,
    setMode: (next) => {
      setMode(next);
      recorder.mode = modeState;
    },
    getStatus,
  };

  if (modeState !== "off") {
    recorder.onRender = ((id, phase, actualDuration, baseDuration, startTime, commitTime) => {
      const shouldSample = modeState !== "heavy" && actualDuration < 16;
      if (shouldSample) return;
      const origin = performance?.timeOrigin ?? Date.now();
      record(
        "react:render",
        {
          id,
          phase,
          actual_duration_ms: Math.round(actualDuration),
          base_duration_ms: Math.round(baseDuration),
          start_ms: Math.round(origin + startTime),
          commit_ms: Math.round(origin + commitTime),
        },
        { level: modeState === "heavy" ? "heavy" : "light" },
      );
    }) as ProfilerOnRenderCallback;
  }

  globalAny.__CTX_WAL__ = recorder;
  installHooks(recorder, () => modeState);
  initPerformanceObservers(recorder, () => modeState);

  recorder.record("wal:init", {
    mode: modeState,
    url: typeof window !== "undefined" ? window.location.href : "",
    user_agent: typeof navigator !== "undefined" ? navigator.userAgent : "unknown",
  });

  return recorder;
};
