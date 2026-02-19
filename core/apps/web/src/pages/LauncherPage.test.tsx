import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import LauncherPage from "./LauncherPage";
import { loadLauncherRecents, upsertLauncherRecent } from "../state/launcherRecentsStore";
import {
  createWorkspace,
  getHealth,
  idToString,
  listWorkspaces,
} from "../api/client";
import {
  desktopConnectLocal,
  desktopGetConnection,
  isDesktopApp,
} from "../utils/desktop";

const navigateMock = vi.hoisted(() => vi.fn());

vi.mock("react-router-dom", async () => {
  const actual = await vi.importActual<typeof import("react-router-dom")>("react-router-dom");
  return {
    ...actual,
    useNavigate: () => navigateMock,
  };
});

vi.mock("../api/client", async () => {
  const actual = await vi.importActual<typeof import("../api/client")>("../api/client");
  return {
    ...actual,
    applyDaemonDesktopConnection: vi.fn(),
    createWorkspace: vi.fn(),
    getHealth: vi.fn(),
    idToString: vi.fn((value: unknown) => String(value ?? "")),
    listWorkspaces: vi.fn(),
  };
});

vi.mock("../utils/desktop", async () => {
  const actual = await vi.importActual<typeof import("../utils/desktop")>("../utils/desktop");
  return {
    ...actual,
    desktopConnectLocal: vi.fn(),
    desktopGetConnection: vi.fn(),
    isDesktopApp: vi.fn(),
  };
});

vi.mock("../state/launcherRecentsStore", () => ({
  loadLauncherRecents: vi.fn(),
  upsertLauncherRecent: vi.fn(),
}));

describe("LauncherPage recents", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    navigateMock.mockReset();
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(desktopGetConnection).mockResolvedValue({ kind: "none" });
    vi.mocked(loadLauncherRecents).mockResolvedValue([]);
    vi.mocked(upsertLauncherRecent).mockResolvedValue([]);
    vi.mocked(getHealth).mockResolvedValue({
      daemon_version: "0.0.0-test",
      compatibility: { desktop_exact_version: "0.0.0-test", mobile_api_min: 1, mobile_api_max: 1 },
    } as never);
    vi.mocked(idToString).mockImplementation((value: unknown) => String(value ?? ""));
  });

  it("renders recents loaded from launcher recents store", async () => {
    vi.mocked(loadLauncherRecents).mockResolvedValueOnce([
      {
        kind: "local",
        label: "ctx-monorepo",
        root_path: "/Users/example-user/code/ctx-monorepo",
        updated_at_ms: 1000,
      },
    ]);

    render(<LauncherPage />);

    expect(await screen.findByText("ctx-monorepo")).toBeInTheDocument();
    expect(screen.getByText("Local: /Users/example-user/code/ctx-monorepo")).toBeInTheDocument();
    expect(loadLauncherRecents).toHaveBeenCalled();
  });

  it("upserts recents when opening a local recent workspace", async () => {
    vi.mocked(loadLauncherRecents).mockResolvedValueOnce([
      {
        kind: "local",
        label: "repo-a",
        root_path: "/tmp/repo-a",
        updated_at_ms: 50,
      },
    ]);
    vi.mocked(desktopConnectLocal).mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4399",
      token: "test-token",
    } as never);
    vi.mocked(listWorkspaces).mockResolvedValue([]);
    vi.mocked(createWorkspace).mockResolvedValue({ id: "ws-1" } as never);

    render(<LauncherPage />);

    fireEvent.click(await screen.findByRole("button", { name: /repo-a/i }));

    await waitFor(() => {
      expect(upsertLauncherRecent).toHaveBeenCalledWith(expect.objectContaining({
        kind: "local",
        root_path: "/tmp/repo-a",
        label: "repo-a",
        updated_at_ms: expect.any(Number),
      }));
      expect(navigateMock).toHaveBeenCalledWith("/workspaces/ws-1", { replace: true });
    });
  });
});
