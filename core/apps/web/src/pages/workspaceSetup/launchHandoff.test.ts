import { describe, expect, it } from "vitest";
import { launchErrorFromSnapshot } from "./launchHandoff";

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
});
