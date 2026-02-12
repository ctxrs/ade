export type UiDiagnosticSeverity = "info" | "warning" | "error";

export type UiDiagnosticEvent = {
  id: number;
  ts: number;
  source: string;
  code: string;
  severity: UiDiagnosticSeverity;
  message: string;
  fatal?: boolean;
  context?: Record<string, unknown>;
};

export type UiDiagnosticInput = {
  source: string;
  code: string;
  message: string;
  severity?: UiDiagnosticSeverity;
  fatal?: boolean;
  context?: Record<string, unknown>;
};

const DEFAULT_MAX_EVENTS = 200;
let maxEvents = DEFAULT_MAX_EVENTS;
let nextId = 1;
let events: UiDiagnosticEvent[] = [];
const listeners = new Set<() => void>();
let runtimeHandlersInstalled = false;
let runtimeHandlersCleanup: (() => void) | null = null;

const notifyListeners = () => {
  for (const listener of listeners) {
    listener();
  }
};

export const normalizeDiagnosticErrorMessage = (value: unknown, fallback = "Unknown error"): string => {
  if (value instanceof Error) {
    return value.message || fallback;
  }
  if (typeof value === "string" && value.trim().length > 0) {
    return value.trim();
  }
  if (value && typeof value === "object") {
    const maybeMessage = (value as Record<string, unknown>).message;
    if (typeof maybeMessage === "string" && maybeMessage.trim().length > 0) {
      return maybeMessage.trim();
    }
    try {
      return JSON.stringify(value);
    } catch {
      return fallback;
    }
  }
  return fallback;
};

export const emitUiDiagnostic = (input: UiDiagnosticInput): UiDiagnosticEvent => {
  const event: UiDiagnosticEvent = {
    id: nextId++,
    ts: Date.now(),
    source: String(input.source || "unknown"),
    code: String(input.code || "unknown"),
    severity: input.severity ?? "error",
    message: String(input.message || "Unknown error"),
    fatal: input.fatal === true ? true : undefined,
    context: input.context,
  };
  events = [...events, event];
  if (events.length > maxEvents) {
    events = events.slice(events.length - maxEvents);
  }
  notifyListeners();
  return event;
};

export const getUiDiagnostics = (): UiDiagnosticEvent[] => [...events];

export const clearUiDiagnostics = () => {
  if (events.length === 0) return;
  events = [];
  notifyListeners();
};

export const subscribeUiDiagnostics = (listener: () => void): (() => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};

export const installGlobalRuntimeDiagnosticHandlers = () => {
  if (runtimeHandlersInstalled) return;
  if (typeof window === "undefined") return;
  const onError = (event: ErrorEvent) => {
    const message = normalizeDiagnosticErrorMessage(event.error ?? event.message ?? "Unhandled runtime error");
    const isResizeObserverLoop =
      message.includes("ResizeObserver loop completed with undelivered notifications") ||
      message.includes("ResizeObserver loop limit exceeded");
    emitUiDiagnostic({
      source: "runtime",
      code: isResizeObserverLoop ? "runtime.resize_observer_loop" : "runtime.error",
      severity: isResizeObserverLoop ? "warning" : "error",
      message,
      context: {
        filename: event.filename,
        lineno: event.lineno,
        colno: event.colno,
      },
    });
  };
  const onUnhandledRejection = (event: PromiseRejectionEvent) => {
    emitUiDiagnostic({
      source: "runtime",
      code: "runtime.unhandled_rejection",
      severity: "error",
      message: normalizeDiagnosticErrorMessage(event.reason, "Unhandled promise rejection"),
    });
  };
  window.addEventListener("error", onError);
  window.addEventListener("unhandledrejection", onUnhandledRejection);
  runtimeHandlersCleanup = () => {
    window.removeEventListener("error", onError);
    window.removeEventListener("unhandledrejection", onUnhandledRejection);
    runtimeHandlersCleanup = null;
    runtimeHandlersInstalled = false;
  };
  runtimeHandlersInstalled = true;
};

export const resetUiDiagnosticsForTests = () => {
  clearUiDiagnostics();
  maxEvents = DEFAULT_MAX_EVENTS;
  nextId = 1;
  if (runtimeHandlersCleanup) {
    runtimeHandlersCleanup();
  }
};

export const setUiDiagnosticsMaxEventsForTests = (next: number) => {
  maxEvents = Math.max(1, Math.trunc(next));
  if (events.length > maxEvents) {
    events = events.slice(events.length - maxEvents);
  }
};
