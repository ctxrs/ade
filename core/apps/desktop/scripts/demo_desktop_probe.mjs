#!/usr/bin/env node

export function screenPointFromProbe(metrics, rect) {
  if (!metrics || !rect) {
    throw new Error("metrics and rect are required");
  }
  const globalX = Number(metrics.screenX) + Number(rect.left) + Number(rect.width) / 2;
  const windowTop = Number(metrics.windowInnerPosition?.y ?? metrics.windowOuterPosition?.y ?? metrics.screenY ?? 0);
  const globalY = windowTop + Number(rect.top) + Number(rect.height) / 2;
  return {
    x: Number(globalX.toFixed(2)),
    y: Number(globalY.toFixed(2)),
  };
}

export function buildPromptScenario(point, promptText, opts = {}) {
  const actions = [
    { kind: "move", x: point.x, y: point.y, duration_ms: opts.moveDurationMs ?? 550 },
    { kind: "click", button: "left" },
    { kind: "type", text: promptText, cps: opts.cps ?? 14 },
  ];
  if (opts.submitViaKey !== false) {
    actions.push({ kind: "key", name: opts.submitKeyName ?? "return" });
  }
  actions.push({ kind: "wait", duration_ms: opts.waitDurationMs ?? 300 });
  return {
    actions,
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
