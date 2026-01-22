export type LoadTestTelemetrySnapshot = {
  session_switches: Array<{
    from_session_id: string | null;
    to_session_id: string | null;
    started_at_ms: number;
    completed_at_ms?: number;
    duration_ms?: number;
    status: "completed" | "abandoned";
  }>;
  long_tasks: Array<{
    start_ms: number;
    duration_ms: number;
  }>;
  memory_samples: Array<{
    at_ms: number;
    used_js_heap_size?: number;
    total_js_heap_size?: number;
    js_heap_size_limit?: number;
  }>;
  meta: {
    time_origin_ms: number;
    user_agent: string;
  };
};

type PendingSessionSwitch = {
  from_session_id: string | null;
  to_session_id: string | null;
  started_at_ms: number;
};

type LoadTestTelemetry = {
  enabled: boolean;
  startSessionSwitch: (fromSessionId: string | null, toSessionId: string | null) => void;
  finishSessionSwitch: (toSessionId: string | null) => void;
  getSnapshot: () => LoadTestTelemetrySnapshot;
  reset: () => void;
  stop: () => void;
};

const MAX_ENTRIES = 2000;
const MEMORY_SAMPLE_MS = 2000;

let telemetry: LoadTestTelemetry | null = null;

const nowMs = (): number => {
  if (typeof performance !== "undefined" && typeof performance.now === "function") {
    return (performance.timeOrigin ?? Date.now()) + performance.now();
  }
  return Date.now();
};

const shouldEnable = (): boolean => {
  if (typeof window === "undefined") return false;
  const query = new URLSearchParams(window.location.search);
  const queryEnabled = query.get("loadtest") === "1";
  const flagEnabled = Boolean((window as any).__CTX_LOAD_TEST__);
  return Boolean(import.meta.env.DEV || import.meta.env.MODE === "test" || queryEnabled || flagEnabled);
};

export const initLoadTestTelemetry = (): LoadTestTelemetry | null => {
  if (telemetry) return telemetry;
  if (!shouldEnable()) return null;

  const session_switches: LoadTestTelemetrySnapshot["session_switches"] = [];
  const long_tasks: LoadTestTelemetrySnapshot["long_tasks"] = [];
  const memory_samples: LoadTestTelemetrySnapshot["memory_samples"] = [];
  const meta: LoadTestTelemetrySnapshot["meta"] = {
    time_origin_ms:
      typeof performance !== "undefined" && typeof performance.timeOrigin === "number"
        ? performance.timeOrigin
        : Date.now(),
    user_agent: typeof navigator !== "undefined" ? navigator.userAgent : "unknown",
  };

  let pending: PendingSessionSwitch | null = null;
  let observer: PerformanceObserver | null = null;
  let memoryTimer: number | null = null;

  const pushWithLimit = <T>(list: T[], entry: T) => {
    if (list.length >= MAX_ENTRIES) list.shift();
    list.push(entry);
  };

  const recordLongTask = (entry: PerformanceEntry) => {
    const start_ms = meta.time_origin_ms + (entry.startTime ?? 0);
    const duration_ms = entry.duration ?? 0;
    pushWithLimit(long_tasks, { start_ms, duration_ms });
  };

  const sampleMemory = () => {
    if (typeof performance === "undefined") return;
    const mem = (performance as Performance & {
      memory?: { usedJSHeapSize: number; totalJSHeapSize: number; jsHeapSizeLimit: number };
    }).memory;
    if (!mem || typeof mem.usedJSHeapSize !== "number") return;
    pushWithLimit(memory_samples, {
      at_ms: nowMs(),
      used_js_heap_size: mem.usedJSHeapSize,
      total_js_heap_size: mem.totalJSHeapSize,
      js_heap_size_limit: mem.jsHeapSizeLimit,
    });
  };

  if (typeof PerformanceObserver !== "undefined") {
    const types = (PerformanceObserver as any).supportedEntryTypes as string[] | undefined;
    if (types?.includes("longtask")) {
      observer = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
          recordLongTask(entry);
        }
      });
      try {
        observer.observe({ entryTypes: ["longtask"] });
      } catch {
        observer = null;
      }
    }
  }

  sampleMemory();
  if (typeof window !== "undefined" && typeof window.setInterval === "function") {
    memoryTimer = window.setInterval(sampleMemory, MEMORY_SAMPLE_MS);
  }

  telemetry = {
    enabled: true,
    startSessionSwitch: (fromSessionId, toSessionId) => {
      if (pending) {
        pushWithLimit(session_switches, { ...pending, status: "abandoned" });
      }
      pending = {
        from_session_id: fromSessionId,
        to_session_id: toSessionId,
        started_at_ms: nowMs(),
      };
    },
    finishSessionSwitch: (toSessionId) => {
      if (!pending) return;
      if (pending.to_session_id && toSessionId && pending.to_session_id !== toSessionId) return;
      const completed_at_ms = nowMs();
      const duration_ms = completed_at_ms - pending.started_at_ms;
      pushWithLimit(session_switches, {
        ...pending,
        completed_at_ms,
        duration_ms,
        status: "completed",
      });
      pending = null;
    },
    getSnapshot: () => ({
      session_switches: session_switches.slice(),
      long_tasks: long_tasks.slice(),
      memory_samples: memory_samples.slice(),
      meta,
    }),
    reset: () => {
      session_switches.length = 0;
      long_tasks.length = 0;
      memory_samples.length = 0;
      pending = null;
      sampleMemory();
    },
    stop: () => {
      observer?.disconnect();
      observer = null;
      if (memoryTimer !== null && typeof window !== "undefined") {
        window.clearInterval(memoryTimer);
        memoryTimer = null;
      }
    },
  };

  if (typeof window !== "undefined") {
    (window as any).__ctxLoadTestTelemetry = {
      enabled: true,
      getSnapshot: telemetry.getSnapshot,
      reset: telemetry.reset,
      stop: telemetry.stop,
    };
  }

  return telemetry;
};

export const getLoadTestTelemetry = (): LoadTestTelemetry | null => telemetry;
