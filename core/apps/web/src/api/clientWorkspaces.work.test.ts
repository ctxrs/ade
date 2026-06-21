import { beforeEach, describe, expect, it, vi } from "vitest";

const { apiAnyMock } = vi.hoisted(() => ({
  apiAnyMock: vi.fn(),
}));

vi.mock("./clientBase", async () => {
  const actual = await vi.importActual<typeof import("./clientBase")>("./clientBase");
  return {
    ...actual,
    apiAny: apiAnyMock,
  };
});

import {
  getWorkspaceWork,
  getWorkspaceWorkContext,
  getWorkspaceWorkEvidence,
  getWorkspaceWorkInspector,
  getWorkspaceWorkReport,
  getWorkspaceWorkTimeline,
  listWorkspaceWork,
} from "./clientWorkspaces";

describe("workspace Work API client", () => {
  beforeEach(() => {
    apiAnyMock.mockReset();
    apiAnyMock.mockResolvedValue({});
  });

  it("builds stable Work route URLs", async () => {
    await listWorkspaceWork("ws 1", { limit: 25 });
    await getWorkspaceWork("ws 1", "wrk/a");
    await getWorkspaceWorkReport("ws 1", "wrk/a");
    await getWorkspaceWorkInspector("ws 1", "wrk/a");
    await getWorkspaceWorkContext("ws 1", "wrk/a", { budget: 4000 });
    await getWorkspaceWorkTimeline("ws 1", "wrk/a", { limit: 10 });
    await getWorkspaceWorkEvidence("ws 1", "wrk/a");

    expect(apiAnyMock.mock.calls.map(([path]) => path)).toEqual([
      "/api/workspaces/ws%201/work?limit=25",
      "/api/workspaces/ws%201/work/wrk%2Fa",
      "/api/workspaces/ws%201/work/wrk%2Fa/report",
      "/api/workspaces/ws%201/work/wrk%2Fa/inspector",
      "/api/workspaces/ws%201/work/wrk%2Fa/context?budget=4000",
      "/api/workspaces/ws%201/work/wrk%2Fa/timeline?limit=10",
      "/api/workspaces/ws%201/work/wrk%2Fa/evidence",
    ]);
  });
});
