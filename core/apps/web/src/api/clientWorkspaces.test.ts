import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./daemonConnection", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./daemonConnection")>();
  return {
    ...actual,
    getDaemonConnection: vi.fn(() => ({
      authToken: null,
    })),
    getDaemonWsUrl: vi.fn((path: string, query?: URLSearchParams) => {
      const qs = query?.toString();
      return qs ? `ws://daemon.test${path}?${qs}` : `ws://daemon.test${path}`;
    }),
  };
});

import { buildExecutionLaunchWsUrl } from "./clientWorkspaces";
import { getDaemonConnection, getDaemonWsUrl } from "./daemonConnection";

describe("clientWorkspaces websocket urls", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("builds execution launch stream URL on canonical daemon host with token", () => {
    vi.mocked(getDaemonConnection).mockReturnValueOnce({
      authToken: "token-1",
    } as any);

    const url = buildExecutionLaunchWsUrl("job-1");
    expect(url).toContain("ws://daemon.test/api/execution/launch/stream");
    expect(url).toContain("job_id=job-1");
    expect(url).toContain("token=token-1");
    expect(vi.mocked(getDaemonWsUrl)).toHaveBeenCalledTimes(1);
  });

  it("builds execution launch stream URL without token when auth is absent", () => {
    vi.mocked(getDaemonConnection).mockReturnValueOnce({
      authToken: null,
    } as any);

    const url = buildExecutionLaunchWsUrl("job-2");
    expect(url).toContain("ws://daemon.test/api/execution/launch/stream");
    expect(url).toContain("job_id=job-2");
    expect(url).not.toContain("token=");
    expect(vi.mocked(getDaemonWsUrl)).toHaveBeenCalledTimes(1);
  });
});
