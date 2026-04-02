import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { tmpdir } from "node:os";

import {
  ensureWorkspaceRoot,
  isoMinutesAgo,
  normalizeDemoTaskStatus,
  patchActiveSnapshotTurnsForDemoStatus,
  resolveWorkspaceDbPath,
  resolveWorkspaceTemplateDir,
} from "./demo_fixture_bootstrap.mjs";

test("resolveWorkspaceTemplateDir resolves template paths relative to the scenario file", () => {
  const scenarioPath = "/tmp/demo-fixtures/scenario.json";
  const resolved = resolveWorkspaceTemplateDir(scenarioPath, {
    workspace_template_dir: "../workspace-template",
  });
  assert.equal(resolved, "/tmp/workspace-template");
});

test("ensureWorkspaceRoot materializes template content and resets stray files", () => {
  const dir = mkdtempSync(path.join(tmpdir(), "demo-fixture-bootstrap-"));
  const templateDir = path.join(dir, "template");
  const workspaceRoot = path.join(dir, "workspace");
  const scenarioPath = path.join(dir, "scenario.json");

  try {
    execFileSync("git", ["config", "--global", "init.defaultBranch", "main"], { stdio: "ignore" });
  } catch {
    // Ignore local git defaults if the environment disallows global config writes.
  }

  try {
    writeFileSync(scenarioPath, "{}", "utf8");
    mkdirSync(path.join(templateDir, "src"), { recursive: true });
    writeFileSync(path.join(templateDir, "README.md"), "# Template\n", "utf8");
    writeFileSync(path.join(templateDir, "src", "demo.txt"), "seeded\n", "utf8");

    const scenario = {
      workspace_name: "HN Mobile",
      workspace_template_dir: "./template",
      workspace_commit_message: "seed baseline",
    };

    ensureWorkspaceRoot(workspaceRoot, scenario, scenarioPath);
    assert.equal(readFileSync(path.join(workspaceRoot, "src", "demo.txt"), "utf8"), "seeded\n");
    assert.ok(existsSync(path.join(workspaceRoot, ".git")));
    assert.equal(
      execFileSync("git", ["log", "-1", "--pretty=%s"], { cwd: workspaceRoot, encoding: "utf8" }).trim(),
      "seed baseline",
    );

    writeFileSync(path.join(workspaceRoot, "stray.txt"), "remove me\n", "utf8");
    ensureWorkspaceRoot(workspaceRoot, scenario, scenarioPath);

    assert.equal(existsSync(path.join(workspaceRoot, "stray.txt")), false);
    assert.equal(readFileSync(path.join(workspaceRoot, "src", "demo.txt"), "utf8"), "seeded\n");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("resolveWorkspaceDbPath points at the workspace sqlite file", () => {
  assert.equal(
    resolveWorkspaceDbPath("/tmp/ctx-daemon", "workspace-123"),
    "/tmp/ctx-daemon/db/workspaces/workspace-123/db.sqlite",
  );
});

test("normalizeDemoTaskStatus accepts supported demo row states", () => {
  assert.equal(normalizeDemoTaskStatus("working"), "working");
  assert.equal(normalizeDemoTaskStatus("new_message"), "new_message");
  assert.equal(normalizeDemoTaskStatus("unread"), "new_message");
  assert.equal(normalizeDemoTaskStatus("idle"), "idle");
  assert.throws(() => normalizeDemoTaskStatus("failed"), /unsupported demo task status/);
});

test("isoMinutesAgo offsets the supplied base time", () => {
  const baseNow = new Date("2026-03-22T18:00:00.000Z");
  assert.equal(isoMinutesAgo(17, baseNow), "2026-03-22T17:43:00.000Z");
  assert.throws(() => isoMinutesAgo(-1, baseNow), /minutes_ago must be a non-negative number/);
});

test("patchActiveSnapshotTurnsForDemoStatus marks the last seeded turn as running for working rows", () => {
  const turnsJson = JSON.stringify([
    {
      turn_id: "turn-1",
      status: "completed",
      start_seq: 4,
      end_seq: 8,
      updated_at: "2026-04-01T20:57:23.459Z",
    },
  ]);

  const patched = JSON.parse(
    patchActiveSnapshotTurnsForDemoStatus(turnsJson, "working", "2026-04-01T21:00:23.552Z"),
  );

  assert.equal(patched[0]?.status, "running");
  assert.equal(patched[0]?.end_seq, null);
  assert.equal(patched[0]?.updated_at, "2026-04-01T21:00:23.552Z");
});

test("patchActiveSnapshotTurnsForDemoStatus leaves non-working rows unchanged", () => {
  const turnsJson = JSON.stringify([
    {
      turn_id: "turn-1",
      status: "completed",
      end_seq: 8,
      updated_at: "2026-04-01T20:57:23.459Z",
    },
  ]);

  assert.equal(
    patchActiveSnapshotTurnsForDemoStatus(turnsJson, "idle", "2026-04-01T21:00:23.552Z"),
    turnsJson,
  );
});
