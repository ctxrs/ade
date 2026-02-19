import { render, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import WorkbenchPage from "./WorkbenchPage";

const { trackWorkspaceOpenedMock, trackFeatureUsedMock, isDesktopAppMock, desktopGetConnectionMock } =
  vi.hoisted(() => ({
    trackWorkspaceOpenedMock: vi.fn(),
    trackFeatureUsedMock: vi.fn(),
    isDesktopAppMock: vi.fn(),
    desktopGetConnectionMock: vi.fn(),
  }));

vi.mock("../utils/analytics", async () => {
  const actual = await vi.importActual<typeof import("../utils/analytics")>("../utils/analytics");
  return {
    ...actual,
    trackWorkspaceOpened: trackWorkspaceOpenedMock,
    trackFeatureUsed: trackFeatureUsedMock,
  };
});

vi.mock("../utils/desktop", async () => {
  const actual = await vi.importActual<typeof import("../utils/desktop")>("../utils/desktop");
  return {
    ...actual,
    isDesktopApp: isDesktopAppMock,
    desktopGetConnection: desktopGetConnectionMock,
  };
});

vi.mock("../workbench/store", () => ({
  WorkbenchStoreProvider: ({ children }: { children: ReactNode }) => <>{children}</>,
}));

vi.mock("../state/workspaceActiveSnapshotStore", () => ({
  WorkspaceActiveSnapshotProvider: ({ children }: { children: ReactNode }) => <>{children}</>,
}));

vi.mock("./WorkbenchPage.shell", () => ({
  WorkbenchPageInner: () => <div data-testid="workbench-page-inner" />,
}));

const renderWorkbenchPage = () =>
  render(
    <MemoryRouter initialEntries={["/workspaces/ws-1"]}>
      <Routes>
        <Route path="/workspaces/:id" element={<WorkbenchPage />} />
      </Routes>
    </MemoryRouter>,
  );

describe("WorkbenchPage analytics", () => {
  beforeEach(() => {
    trackWorkspaceOpenedMock.mockReset();
    trackFeatureUsedMock.mockReset();
    isDesktopAppMock.mockReset();
    desktopGetConnectionMock.mockReset();
  });

  it("tracks local workspace opened when not running in desktop", async () => {
    isDesktopAppMock.mockReturnValue(false);

    renderWorkbenchPage();

    await waitFor(() => {
      expect(trackWorkspaceOpenedMock).toHaveBeenCalledWith("local");
    });
    expect(trackFeatureUsedMock).toHaveBeenCalledWith("workbench_opened", { workspace_kind: "local" });
  });

  it("tracks remote workspace opened when connected through ssh desktop daemon", async () => {
    isDesktopAppMock.mockReturnValue(true);
    desktopGetConnectionMock.mockResolvedValue({ kind: "ssh" });

    renderWorkbenchPage();

    await waitFor(() => {
      expect(trackWorkspaceOpenedMock).toHaveBeenCalledWith("remote");
    });
    expect(trackFeatureUsedMock).toHaveBeenCalledWith("workbench_opened", { workspace_kind: "remote" });
  });
});
