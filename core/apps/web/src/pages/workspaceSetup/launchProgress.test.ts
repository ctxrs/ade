import { describe, expect, it } from "vitest";
import { formatLaunchTime, mergeLaunchLogs } from "./launchProgress";

describe("launchProgress", () => {
  it("preformats and appends monotonic launch-log batches", () => {
    const merged = mergeLaunchLogs([], [
      {
        seq: 10,
        ts: "2026-03-09T19:10:24Z",
        phase: "machine_check",
        level: "info",
        message: "checking container runtime",
      },
      {
        seq: 11,
        ts: "2026-03-09T19:10:25Z",
        phase: "machine_start_or_init",
        level: "info",
        message: "starting machine",
      },
    ]);

    expect(merged.map((line) => line.seq)).toEqual([10, 11]);
    expect(merged[0].phaseLabel).toBe("Machine check");
    expect(merged[0].timeLabel).toBe(formatLaunchTime("2026-03-09T19:10:24Z"));
    expect(merged[1].phaseLabel).toBe("Machine start/init");
    expect(merged[1].timeLabel).toBe(formatLaunchTime("2026-03-09T19:10:25Z"));
  });

  it("dedupes and resorts overlapping snapshot batches", () => {
    const current = mergeLaunchLogs([], [
      {
        seq: 2,
        ts: "2026-03-09T19:10:25Z",
        phase: "machine_start_or_init",
        level: "info",
        message: "starting machine",
      },
      {
        seq: 3,
        ts: "2026-03-09T19:10:26Z",
        phase: "machine_start_or_init",
        level: "info",
        message: "machine ready",
      },
    ]);

    const merged = mergeLaunchLogs(current, [
      {
        seq: 1,
        ts: "2026-03-09T19:10:24Z",
        phase: "machine_check",
        level: "info",
        message: "checking container runtime",
      },
      {
        seq: 3,
        ts: "2026-03-09T19:10:26Z",
        phase: "machine_start_or_init",
        level: "warn",
        message: "machine ready (retry)",
      },
    ]);

    expect(merged.map((line) => [line.seq, line.level, line.message])).toEqual([
      [1, "info", "checking container runtime"],
      [2, "info", "starting machine"],
      [3, "warn", "machine ready (retry)"],
    ]);
  });

  it("trims launch logs to the most recent 400 lines", () => {
    const incoming = Array.from({ length: 405 }, (_, index) => ({
      seq: index + 1,
      ts: "2026-03-09T19:10:24Z",
      phase: "machine_check" as const,
      level: "info" as const,
      message: `line ${index + 1}`,
    }));

    const merged = mergeLaunchLogs([], incoming);

    expect(merged).toHaveLength(400);
    expect(merged[0]?.seq).toBe(6);
    expect(merged.at(-1)?.seq).toBe(405);
  });
});
