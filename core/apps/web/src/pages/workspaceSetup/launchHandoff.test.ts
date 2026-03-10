import { describe, expect, it } from "vitest";
import { createLaunchLogBatcher, launchErrorFromSnapshot } from "./launchHandoff";

describe("launchHandoff", () => {
  it("preserves the current launch phase in extracted error messages", () => {
    expect(launchErrorFromSnapshot({
      job_id: "job_123",
      workspace_id: "ws_123",
      kind: "workspace_launch",
      state: "error",
      created_at: "2026-03-09T00:00:00Z",
      started_at: "2026-03-09T00:00:01Z",
      current_phase: "machine_start_or_init",
      error: "failed to start remote daemon",
      phases: [],
      logs: [],
    })).toBe("Machine start/init: failed to start remote daemon");
  });

  it("batches launch-log lines before flushing them", () => {
    const appended: number[][] = [];
    const scheduler: { scheduledFlush: (() => void) | null } = { scheduledFlush: null };
    const batcher = createLaunchLogBatcher(
      (lines) => appended.push(lines.map((line) => line.seq)),
      {
        scheduleFlush: (flush) => {
          scheduler.scheduledFlush = flush;
          return 1;
        },
        cancelFlush: () => {
          scheduler.scheduledFlush = null;
        },
      },
    );

    batcher.enqueue({
      seq: 1,
      ts: "2026-03-09T00:00:01Z",
      phase: "machine_check",
      level: "info",
      message: "checking",
    });
    batcher.enqueue({
      seq: 2,
      ts: "2026-03-09T00:00:02Z",
      phase: "machine_check",
      level: "info",
      message: "still checking",
    });

    expect(appended).toEqual([]);
    expect(scheduler.scheduledFlush).not.toBeNull();
    if (!scheduler.scheduledFlush) {
      throw new Error("expected a scheduled flush");
    }
    scheduler.scheduledFlush();

    expect(appended).toEqual([[1, 2]]);
  });

  it("flushes pending launch-log lines immediately when requested", () => {
    const appended: number[][] = [];
    let canceled = false;
    const batcher = createLaunchLogBatcher(
      (lines) => appended.push(lines.map((line) => line.seq)),
      {
        scheduleFlush: () => 7,
        cancelFlush: () => {
          canceled = true;
        },
      },
    );

    batcher.enqueue({
      seq: 3,
      ts: "2026-03-09T00:00:03Z",
      phase: "machine_start_or_init",
      level: "info",
      message: "starting",
    });

    batcher.flush();

    expect(canceled).toBe(true);
    expect(appended).toEqual([[3]]);
  });
});
