import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";

const SCRIPT_PATH = path.resolve("core/apps/desktop/scripts/macos_demo_conductor.swift");

test("macOS conductor dry-run clicks at the last moved cursor point", () => {
  const tempDir = mkdtempSync(path.join(os.tmpdir(), "ctx-macos-demo-conductor-"));
  try {
    const scenarioPath = path.join(tempDir, "scenario.json");
    writeFileSync(
      scenarioPath,
      `${JSON.stringify({
        actions: [
          { kind: "move", x: 321.5, y: 654.25, duration_ms: 120 },
          { kind: "click", button: "left" },
        ],
      })}\n`,
      "utf8",
    );
    const result = spawnSync("swift", [SCRIPT_PATH, "--dry-run", "--scenario", scenarioPath], {
      cwd: path.resolve("."),
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr || result.stdout);
    const events = JSON.parse(result.stdout);
    assert.equal(events.length, 2);
    assert.equal(events[0].action, "move");
    assert.deepEqual(events[0].point, { x: 321.5, y: 654.25 });
    assert.equal(events[0].durationMs, 120);
    assert.equal(events[1].action, "click");
    assert.deepEqual(events[1].point, { x: 321.5, y: 654.25 });
    assert.equal(events[1].button, "left");
  } finally {
    rmSync(tempDir, { recursive: true, force: true });
  }
});

test("macOS conductor dry-run uses the seeded initial cursor point for click-only scenarios", () => {
  const tempDir = mkdtempSync(path.join(os.tmpdir(), "ctx-macos-demo-conductor-"));
  try {
    const scenarioPath = path.join(tempDir, "scenario.json");
    writeFileSync(
      scenarioPath,
      `${JSON.stringify({
        initial_cursor_point: { x: 412.75, y: 288.5 },
        actions: [
          { kind: "click", button: "left" },
        ],
      })}\n`,
      "utf8",
    );
    const result = spawnSync("swift", [SCRIPT_PATH, "--dry-run", "--scenario", scenarioPath], {
      cwd: path.resolve("."),
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr || result.stdout);
    const events = JSON.parse(result.stdout);
    assert.equal(events.length, 1);
    assert.equal(events[0].action, "click");
    assert.deepEqual(events[0].point, { x: 412.75, y: 288.5 });
    assert.equal(events[0].button, "left");
  } finally {
    rmSync(tempDir, { recursive: true, force: true });
  }
});
