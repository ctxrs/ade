import { afterEach, describe, expect, it } from "vitest";
import {
  clearUiDiagnostics,
  emitUiDiagnostic,
  getUiDiagnostics,
  installGlobalRuntimeDiagnosticHandlers,
  resetUiDiagnosticsForTests,
  setUiDiagnosticsMaxEventsForTests,
} from "./diagnosticsChannel";

describe("diagnosticsChannel", () => {
  afterEach(() => {
    resetUiDiagnosticsForTests();
  });

  it("stores structured diagnostics with stable ids", () => {
    emitUiDiagnostic({
      source: "api",
      code: "api.http_error",
      message: "request failed",
    });
    emitUiDiagnostic({
      source: "session_supervisor",
      code: "session.load_fatal",
      message: "session failed",
      fatal: true,
    });

    const events = getUiDiagnostics();
    expect(events).toHaveLength(2);
    expect(events[0].id).toBeLessThan(events[1].id);
    expect(events[0].source).toBe("api");
    expect(events[1].fatal).toBe(true);
  });

  it("enforces bounded retention", () => {
    setUiDiagnosticsMaxEventsForTests(2);
    emitUiDiagnostic({ source: "api", code: "a", message: "1" });
    emitUiDiagnostic({ source: "api", code: "b", message: "2" });
    emitUiDiagnostic({ source: "api", code: "c", message: "3" });
    const events = getUiDiagnostics();
    expect(events).toHaveLength(2);
    expect(events[0].code).toBe("b");
    expect(events[1].code).toBe("c");
  });

  it("captures runtime error and unhandled rejection events", () => {
    installGlobalRuntimeDiagnosticHandlers();

    const runtimeError = new ErrorEvent("error", {
      message: "boom",
      filename: "foo.ts",
      lineno: 4,
      colno: 2,
      error: new Error("boom"),
    });
    window.dispatchEvent(runtimeError);

    const rejection = new Event("unhandledrejection") as PromiseRejectionEvent;
    Object.defineProperty(rejection, "reason", {
      value: new Error("nope"),
      configurable: true,
    });
    window.dispatchEvent(rejection);

    const events = getUiDiagnostics();
    expect(events.map((event) => event.code)).toEqual([
      "runtime.error",
      "runtime.unhandled_rejection",
    ]);
  });

  it("clears events", () => {
    emitUiDiagnostic({ source: "api", code: "x", message: "x" });
    clearUiDiagnostics();
    expect(getUiDiagnostics()).toEqual([]);
  });
});
