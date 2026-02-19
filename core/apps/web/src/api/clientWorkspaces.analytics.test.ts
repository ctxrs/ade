import { beforeEach, describe, expect, it, vi } from "vitest";

const { apiAnyMock, trackWorkspaceCreatedMock } = vi.hoisted(() => ({
  apiAnyMock: vi.fn(),
  trackWorkspaceCreatedMock: vi.fn(),
}));

vi.mock("./clientBase", async () => {
  const actual = await vi.importActual<typeof import("./clientBase")>("./clientBase");
  return {
    ...actual,
    apiAny: apiAnyMock,
  };
});

vi.mock("../utils/analytics", async () => {
  const actual = await vi.importActual<typeof import("../utils/analytics")>("../utils/analytics");
  return {
    ...actual,
    trackWorkspaceCreated: trackWorkspaceCreatedMock,
  };
});

import { createWorkspace } from "./clientWorkspaces";

describe("createWorkspace analytics", () => {
  beforeEach(() => {
    apiAnyMock.mockReset();
    trackWorkspaceCreatedMock.mockReset();
  });

  it("tracks local workspace creation by default", async () => {
    apiAnyMock.mockResolvedValue({ id: "ws-1" });

    await createWorkspace("/tmp/repo-a", "Repo A");

    expect(trackWorkspaceCreatedMock).toHaveBeenCalledTimes(1);
    expect(trackWorkspaceCreatedMock).toHaveBeenCalledWith("local");
  });

  it("tracks remote workspace creation when workspace kind is provided", async () => {
    apiAnyMock.mockResolvedValue({ id: "ws-2" });

    await createWorkspace("/tmp/repo-b", "Repo B", "remote");

    expect(trackWorkspaceCreatedMock).toHaveBeenCalledTimes(1);
    expect(trackWorkspaceCreatedMock).toHaveBeenCalledWith("remote");
  });
});
