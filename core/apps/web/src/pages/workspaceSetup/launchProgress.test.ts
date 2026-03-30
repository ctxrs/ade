import { describe, expect, it } from "vitest";
import type { ExecutionLaunchSnapshot } from "../../api/client";
import {
  currentLaunchStepLabel,
  formatLaunchRemaining,
  formatLaunchTime,
  launchEtaRemainingMs,
  mergeLaunchLogs,
} from "./launchProgress";

const baseSnapshot = (): ExecutionLaunchSnapshot => ({
  job_id: "job_123",
  workspace_id: "ws_123",
  kind: "workspace_launch",
  state: "running",
  created_at: "2026-03-10T00:00:00.000Z",
  started_at: "2026-03-10T00:00:00.000Z",
  updated_at: "2026-03-10T00:00:04.000Z",
  current_phase: "artifact_download",
  current_step_label: "downloading required artifacts",
  progress_pct: 12,
  eta_ms: 245000,
  active_download: {
    artifact: "Required artifacts",
    downloaded_bytes: 412 * 1024 * 1024,
    total_bytes: 951 * 1024 * 1024,
    bytes_per_sec: 21 * 1024 * 1024,
  },
  phases: [],
  logs: [],
  error: null,
});

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

  it("prefers the current step label over the coarse phase label", () => {
    expect(currentLaunchStepLabel(baseSnapshot())).toBe("Downloading required artifacts");
  });

  it("counts down remaining time from the latest snapshot timestamp", () => {
    const remainingMs = launchEtaRemainingMs(
      baseSnapshot(),
      Date.parse("2026-03-10T00:00:09.000Z"),
    );
    expect(formatLaunchRemaining(remainingMs)).toBe("4:00 remaining");
  });

  it("keeps a fixed non-download eta while the snapshot is still fresh", () => {
    const snapshot = {
      ...baseSnapshot(),
      current_phase: "machine_start_or_init" as const,
      current_step_label: "waiting for local sandbox runtime readiness",
      active_download: null,
      eta_ms: 18000,
    };
    const remainingMs = launchEtaRemainingMs(
      snapshot,
      Date.parse("2026-03-10T00:00:09.000Z"),
    );
    expect(formatLaunchRemaining(remainingMs)).toBe("18s remaining");
  });

  it("shows estimating when a non-download eta snapshot has gone stale", () => {
    const snapshot = {
      ...baseSnapshot(),
      current_phase: "machine_start_or_init" as const,
      current_step_label: "waiting for local sandbox runtime readiness",
      active_download: null,
      eta_ms: 18000,
    };
    const remainingMs = launchEtaRemainingMs(
      snapshot,
      Date.parse("2026-03-10T00:00:19.000Z"),
    );
    expect(formatLaunchRemaining(remainingMs)).toBe("Estimating remaining…");
  });

  it("shows estimating when a running non-download snapshot has no remaining eta", () => {
    const snapshot = {
      ...baseSnapshot(),
      current_phase: "image_load" as const,
      current_step_label: "loading harness image into local sandbox runtime",
      active_download: null,
      eta_ms: 0,
    };
    expect(formatLaunchRemaining(launchEtaRemainingMs(snapshot, Date.now()))).toBe(
      "Estimating remaining…",
    );
  });
});
