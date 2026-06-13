import { afterEach, describe, expect, it } from "vitest";
import {
  clearSessionMarkdownMeasurementCaches,
  measureSessionMarkdownDocument,
} from "./sessionMarkdownMeasurement";
import type { SessionMarkdownDebugWindow } from "./sessionMarkdownInlineMeasurementDebug";

function clearInlineCodeDebugWindow(): void {
  const debugWindow = window as SessionMarkdownDebugWindow;
  debugWindow.__ctxForceInlineCodeDebug = undefined;
  debugWindow.__ctxInlineCodeDebug = undefined;
  debugWindow.__ctxInlineCodeDebugTarget = undefined;
  debugWindow.__ctxInlineCodeDebugWidth = undefined;
}

describe("sessionMarkdownMeasurement debug", () => {
  afterEach(() => {
    clearInlineCodeDebugWindow();
  });

  it("publishes a structured inline-code debug payload for a matching probe", () => {
    clearSessionMarkdownMeasurementCaches();
    const debugWindow = window as SessionMarkdownDebugWindow;
    debugWindow.__ctxForceInlineCodeDebug = true;
    debugWindow.__ctxInlineCodeDebugTarget = "*";
    debugWindow.__ctxInlineCodeDebugWidth = 588;

    const height = measureSessionMarkdownDocument(
      "Turn fragment composer `sessionMarkdownMeasurement.ts/pretextVirtualizerRowLayout.ts/sessionMarkdownMeasurement.ts/sessionMarkdownMeasurement.ts` `ctx task list` probe command render buffer delta `inline-code/turn-header/sessionThreadDomMeasurement.tsx/sessionThread/web/e2e/core` marker header turn deterministic render virtualizer;",
      588,
    );

    expect(height).toBeGreaterThan(0);
    expect(debugWindow.__ctxInlineCodeDebug).toBeDefined();
    expect(debugWindow.__ctxInlineCodeDebug?.width).toBe(588);
    expect(debugWindow.__ctxInlineCodeDebug?.lines.length).toBeGreaterThan(0);
    expect(debugWindow.__ctxInlineCodeDebug?.items.some((item) => item.kind === "segment")).toBe(
      true,
    );
    expect(debugWindow.__ctxInlineCodeDebug?.startDecisions.length).toBeGreaterThan(0);
  });

  it("does not publish inline-code debug when the width gate does not match", () => {
    const debugWindow = window as SessionMarkdownDebugWindow;
    debugWindow.__ctxForceInlineCodeDebug = true;
    debugWindow.__ctxInlineCodeDebugTarget = "*";
    debugWindow.__ctxInlineCodeDebugWidth = 620;

    measureSessionMarkdownDocument("Debug `ctx task list` probe", 540);

    expect(debugWindow.__ctxInlineCodeDebug).toBeUndefined();
  });

  it("keeps a short command-like inline-code chip whole when it moves to a fresh line after prose", () => {
    clearSessionMarkdownMeasurementCaches();
    const debugWindow = window as SessionMarkdownDebugWindow;
    debugWindow.__ctxForceInlineCodeDebug = true;
    debugWindow.__ctxInlineCodeDebugTarget = "verify:quick";
    debugWindow.__ctxInlineCodeDebugWidth = 788;

    const height = measureSessionMarkdownDocument(
      "This slice is in a good state now: the inline hot path split is green, the viewport scroll/controller extraction is green, and `verify:quick` is back to only the same unrelated Rust hard-cap failures. I’m using one short subagent pass now to reassess what the next highest-value cleanup is after these two reductions, so the next move stays architecture-first instead of random file shaving.",
      788,
    );

    expect(height).toBeGreaterThan(0);
    expect(debugWindow.__ctxInlineCodeDebug).toBeDefined();
    expect(
      debugWindow.__ctxInlineCodeDebug?.lines.some((line) => line.includes("verify:quick")),
    ).toBe(true);
    expect(
      debugWindow.__ctxInlineCodeDebug?.lines.some(
        (line, index, lines) => line.endsWith("verify") && lines[index + 1]?.startsWith(":quick"),
      ),
    ).toBe(false);
  });

  it("does not invent an extra prose wrap after ordinary whitespace inline code", () => {
    clearSessionMarkdownMeasurementCaches();
    const debugWindow = window as SessionMarkdownDebugWindow;
    debugWindow.__ctxForceInlineCodeDebug = true;
    debugWindow.__ctxInlineCodeDebugTarget = "synthetic updater web e2e";
    debugWindow.__ctxInlineCodeDebugWidth = 764;

    const height = measureSessionMarkdownDocument(
      "The `synthetic updater web e2e` sample was recently wrapping at an awkward boundary because the desktop harness examples returned placeholder states and let sample updater behavior enter the measurement path. I adjusted the immediate case, but that exposed fixture fragility.",
      764,
    );

    expect(height).toBeGreaterThan(0);
    expect(debugWindow.__ctxInlineCodeDebug).toBeDefined();
    expect(
      debugWindow.__ctxInlineCodeDebug?.lines.some((line) => line.includes("placeholder")),
    ).toBe(true);
    expect(
      debugWindow.__ctxInlineCodeDebug?.lines.some((line) => line.endsWith("placeholder ")),
    ).toBe(false);
  });

  it("keeps a dotted call continuation whole when a sealed dotted fragment reaches the margin", () => {
    clearSessionMarkdownMeasurementCaches();
    const debugWindow = window as SessionMarkdownDebugWindow;
    debugWindow.__ctxForceInlineCodeDebug = true;
    debugWindow.__ctxInlineCodeDebugTarget = "PipelineController.disconnect()";
    debugWindow.__ctxInlineCodeDebugWidth = 788;

    const height = measureSessionMarkdownDocument(
      "In a synthetic layout note, every `TimeoutReached` signal called the shared `PipelineController.disconnect()`. The paragraph keeps the dotted method call near the wrap boundary so `disconnect()` stays attached to the call instead of splitting after the dot.",
      788,
    );

    expect(height).toBeGreaterThan(0);
    expect(debugWindow.__ctxInlineCodeDebug).toBeDefined();
    expect(
      debugWindow.__ctxInlineCodeDebug?.lines.some((line) => line.includes("disconnect()")),
    ).toBe(true);
    expect(
      debugWindow.__ctxInlineCodeDebug?.lines.some(
        (line, index, lines) =>
          line.endsWith("PipelineController.") && lines[index + 1]?.startsWith("disconnect()"),
      ),
    ).toBe(false);
  });

  it("keeps shorter dotted call siblings whole under the same continuation rule", () => {
    clearSessionMarkdownMeasurementCaches();
    const debugWindow = window as SessionMarkdownDebugWindow;
    debugWindow.__ctxForceInlineCodeDebug = true;
    debugWindow.__ctxInlineCodeDebugTarget = "observer.disconnect()";
    debugWindow.__ctxInlineCodeDebugWidth = 620;

    const height = measureSessionMarkdownDocument(
      "The observer stays healthy when `observer.disconnect()` moves as one chip before the trailing prose explains why the restart no longer interrupts the session unexpectedly.",
      620,
    );

    expect(height).toBeGreaterThan(0);
    expect(debugWindow.__ctxInlineCodeDebug).toBeDefined();
    expect(
      debugWindow.__ctxInlineCodeDebug?.lines.some((line) => line.includes("disconnect()")),
    ).toBe(true);
    expect(
      debugWindow.__ctxInlineCodeDebug?.lines.some(
        (line, index, lines) => line.endsWith("observer.") && lines[index + 1]?.startsWith("disconnect()"),
      ),
    ).toBe(false);
  });
});
