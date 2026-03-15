#!/usr/bin/env node
import test from "node:test";
import assert from "node:assert/strict";

import {
  buildClickScenario,
  buildPromptScenario,
  screenPointFromProbe,
} from "./demo_desktop_probe.mjs";

test("screenPointFromProbe converts CSS rects into bottom-left cursor coordinates", () => {
  const point = screenPointFromProbe(
    {
      screenX: 100,
      screenY: 1120,
      outerHeight: 900,
      innerHeight: 860,
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
    y: 760,
  });
});

test("buildPromptScenario creates the visible prompt playback sequence", () => {
  const scenario = buildPromptScenario({ x: 200, y: 300 }, "Make a ping pong game.");
  assert.deepEqual(scenario.actions.map((action) => action.kind), ["move", "click", "type", "key", "wait"]);
  assert.equal(scenario.actions[2].text, "Make a ping pong game.");
});

test("buildClickScenario creates a simple move-click-wait sequence", () => {
  const scenario = buildClickScenario({ x: 500, y: 600 });
  assert.deepEqual(scenario.actions.map((action) => action.kind), ["move", "click", "wait"]);
});
