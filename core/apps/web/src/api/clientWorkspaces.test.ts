import { beforeEach, describe, expect, it, vi } from "vitest";
import { deriveBrowserStreamToken } from "./browserStreamAuth";

vi.mock("./daemonConnection", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./daemonConnection")>();
  return {
    ...actual,
    getDaemonConnection: vi.fn(() => ({
      baseUrl: null,
      wsBaseUrl: null,
      authToken: null,
      runId: null,
      source: null,
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

  it("builds execution launch stream URL on canonical daemon host with a scoped query token", async () => {
    vi.mocked(getDaemonConnection).mockReturnValueOnce({
      baseUrl: null,
      wsBaseUrl: null,
      authToken: "token-1",
      runId: null,
      source: null,
    });

    const expectedToken = await deriveBrowserStreamToken("token-1", {
      kind: "execution_launch",
      jobId: "job-1",
    });
    const url = await buildExecutionLaunchWsUrl("job-1");
    expect(url).toBe(
      `ws://daemon.test/api/execution/launch/stream?job_id=job-1&token=${expectedToken}`,
    );
    expect(url).not.toContain("token=token-1");
    expect(vi.mocked(getDaemonWsUrl)).toHaveBeenCalledTimes(1);
  });

  it("builds execution launch stream URL without token when auth is absent", async () => {
    vi.mocked(getDaemonConnection).mockReturnValueOnce({
      baseUrl: null,
      wsBaseUrl: null,
      authToken: null,
      runId: null,
      source: null,
    });

    const url = await buildExecutionLaunchWsUrl("job-2");
    expect(url).toContain("ws://daemon.test/api/execution/launch/stream");
    expect(url).toContain("job_id=job-2");
    expect(url).not.toContain("token=");
    expect(vi.mocked(getDaemonWsUrl)).toHaveBeenCalledTimes(1);
  });
});
