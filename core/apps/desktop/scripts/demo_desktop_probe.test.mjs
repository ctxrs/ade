#!/usr/bin/env node
import test from "node:test";
import assert from "node:assert/strict";

import {
  buildClickScenario,
  buildPromptScenario,
  screenPointFromProbe,
} from "./demo_desktop_probe.mjs";

test("screenPointFromProbe converts CSS rects into top-based native cursor coordinates", () => {
  const point = screenPointFromProbe(
    {
      screenX: 100,
      windowInnerPosition: { x: 100, y: 66 },
    },
    {
      left: 20,
      top: 300,
      width: 200,
      height: 40,
    },
  );

  assert.deepEqual(point, {
    x: 220,
    y: 386,
  });
});

test("buildPromptScenario creates the visible prompt playback sequence", () => {
  const scenario = buildPromptScenario({ x: 200, y: 300 }, "Make a ping pong game.");
  assert.deepEqual(scenario.actions.map((action) => action.kind), ["move", "click", "type", "key", "wait"]);
  assert.equal(scenario.actions[2].text, "Make a ping pong game.");
});

test("buildPromptScenario can leave submission to a later scripted step", () => {
  const scenario = buildPromptScenario(
    { x: 200, y: 300 },
    "Add saved stories.",
    { submitViaKey: false, cps: 10, waitDurationMs: 650 },
  );
  assert.deepEqual(scenario.actions.map((action) => action.kind), ["move", "click", "type", "wait"]);
  assert.equal(scenario.actions[2].cps, 10);
  assert.equal(scenario.actions[3].duration_ms, 650);
});

test("buildClickScenario creates a simple move-click-wait sequence", () => {
  const scenario = buildClickScenario({ x: 500, y: 600 });
  assert.deepEqual(scenario.actions.map((action) => action.kind), ["move", "click", "wait"]);
});
