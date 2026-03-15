#!/usr/bin/env node

export function screenPointFromProbe(metrics, rect) {
  if (!metrics || !rect) {
    throw new Error("metrics and rect are required");
  }
  const chromeTop = Math.max(0, Number(metrics.outerHeight) - Number(metrics.innerHeight));
  const globalX = Number(metrics.screenX) + Number(rect.left) + Number(rect.width) / 2;
  // In the automation build on macOS, window.screenY reports the outer window top edge
  // in the same bottom-left coordinate space Quartz expects for native cursor events.
  const globalY = Number(metrics.screenY) - chromeTop - Number(rect.top) - Number(rect.height) / 2;
  return {
    x: Number(globalX.toFixed(2)),
    y: Number(globalY.toFixed(2)),
  };
}

export function buildPromptScenario(point, promptText, opts = {}) {
  return {
    actions: [
      { kind: "move", x: point.x, y: point.y, duration_ms: opts.moveDurationMs ?? 550 },
      { kind: "click", button: "left" },
      { kind: "type", text: promptText, cps: opts.cps ?? 14 },
      { kind: "key", name: "return" },
      { kind: "wait", duration_ms: opts.waitDurationMs ?? 300 },
    ],
  };
}

export function buildClickScenario(point, opts = {}) {
  return {
    actions: [
      { kind: "move", x: point.x, y: point.y, duration_ms: opts.moveDurationMs ?? 450 },
      { kind: "click", button: "left" },
      { kind: "wait", duration_ms: opts.waitDurationMs ?? 300 },
    ],
  };
}
