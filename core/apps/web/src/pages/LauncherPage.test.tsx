import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import LauncherPage from "./LauncherPage";
import { loadLauncherRecents, upsertLauncherRecent } from "../state/launcherRecentsStore";
import {
  getHealth,
  getWorkspaceExecutionConfig,
  idToString,
  listWorkspaces,
  repoStatus,
} from "../api/client";
import {
  desktopConnectLocal,
  desktopConnectSsh,
  desktopGetConnection,
  desktopSetDockRecentLocalWorkspaces,
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
    getHealth: vi.fn(),
    getWorkspaceExecutionConfig: vi.fn(),
    idToString: vi.fn((value: unknown) => String(value ?? "")),
    listWorkspaces: vi.fn(),
    repoStatus: vi.fn(),
  };
});

vi.mock("../utils/desktop", async () => {
  const actual = await vi.importActual<typeof import("../utils/desktop")>("../utils/desktop");
  return {
    ...actual,
    desktopConnectLocal: vi.fn(),
    desktopConnectSsh: vi.fn(),
    desktopGetConnection: vi.fn(),
    desktopSetDockRecentLocalWorkspaces: vi.fn(),
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
    vi.mocked(desktopSetDockRecentLocalWorkspaces).mockResolvedValue();
    vi.mocked(loadLauncherRecents).mockResolvedValue([]);
    vi.mocked(upsertLauncherRecent).mockResolvedValue([]);
    vi.mocked(listWorkspaces).mockResolvedValue([]);
    vi.mocked(repoStatus).mockImplementation((async (req: { path: string }) => ({
      canonical_path: req.path,
      is_repo: true,
    })) as never);
    vi.mocked(getWorkspaceExecutionConfig).mockResolvedValue({ environment: "host" } as never);
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
    expect(screen.getByText("~/code/ctx-monorepo")).toBeInTheDocument();
    expect(loadLauncherRecents).toHaveBeenCalled();
  });

  it("falls back to existing workspaces when persisted launcher recents are empty", async () => {
    vi.mocked(loadLauncherRecents).mockResolvedValueOnce([]);
    vi.mocked(listWorkspaces).mockResolvedValueOnce([
      {
        id: "ws-1",
        name: "ctx-monorepo",
        root_path: "/Users/example-user/code/ctx-monorepo",
        created_at: "2026-03-05T00:00:00.000Z",
      },
    ] as never);

    render(<LauncherPage />);

    expect(await screen.findByText("ctx-monorepo")).toBeInTheDocument();
    expect(screen.getByText("~/code/ctx-monorepo (Host)")).toBeInTheDocument();
  });

  it("does not guess sandbox mode from a local recent path alone", async () => {
    vi.mocked(loadLauncherRecents).mockResolvedValueOnce([
      {
        kind: "local",
        label: "workspace-abc",
        root_path: "/Users/example-user/.ctx/workspaces/staging/workspace-abc",
        updated_at_ms: 1000,
      },
    ]);

    render(<LauncherPage />);

    expect(await screen.findByText("workspace-abc")).toBeInTheDocument();
    expect(screen.getByText("~/.ctx/workspaces/staging/workspace-abc")).toBeInTheDocument();
  });

  it("renders remote host paths with host-mode suffix", async () => {
    vi.mocked(loadLauncherRecents).mockResolvedValueOnce([
      {
        kind: "ssh",
        label: "devbox",
        host: "devbox.example.invalid",
        user: "user",
        remote_port: 4399,
        workspace_root_path: "/home/example-user/code/ctx-monorepo",
        execution_environment: "host",
        updated_at_ms: 1000,
      },
    ]);

    render(<LauncherPage />);

    expect(await screen.findByText("devbox")).toBeInTheDocument();
    expect(screen.getByText("user@devbox.example.invalid:~/code/ctx-monorepo (Host)")).toBeInTheDocument();
  });

  it("renders remote sandbox recents as remote sandboxes", async () => {
    vi.mocked(loadLauncherRecents).mockResolvedValueOnce([
      {
        kind: "ssh",
        label: "sealed-box",
        host: "sealed.example.invalid",
        user: "user",
        remote_port: 4399,
        execution_environment: "sandbox",
        updated_at_ms: 1000,
      },
    ]);

    render(<LauncherPage />);

    expect(await screen.findByText("sealed-box")).toBeInTheDocument();
    expect(screen.getByText("user@sealed.example.invalid (Remote sandbox)")).toBeInTheDocument();
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
    vi.mocked(listWorkspaces).mockResolvedValue([{ id: "ws-1", root_path: "/tmp/repo-a" }] as never);

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

  it("resolves local host recents against canonical workspace paths", async () => {
    vi.mocked(loadLauncherRecents).mockResolvedValueOnce([
      {
        kind: "local",
        label: "repo-canonical",
        root_path: "/tmp/repo-canonical",
        updated_at_ms: 50,
      },
    ]);
    vi.mocked(desktopConnectLocal).mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4399",
      token: "test-token",
    } as never);
    vi.mocked(listWorkspaces).mockResolvedValue([
      { id: "ws-canonical", name: "Canonical Repo", root_path: "/private/tmp/repo-canonical" },
    ] as never);
    vi.mocked(repoStatus).mockResolvedValue({
      canonical_path: "/private/tmp/repo-canonical",
      is_repo: true,
    } as never);

    render(<LauncherPage />);

    fireEvent.click(await screen.findByRole("button", { name: /repo-canonical/i }));

    await waitFor(() => {
      expect(upsertLauncherRecent).toHaveBeenCalledWith(expect.objectContaining({
        kind: "local",
        label: "Canonical Repo",
        root_path: "/private/tmp/repo-canonical",
        updated_at_ms: expect.any(Number),
      }));
      expect(navigateMock).toHaveBeenCalledWith("/workspaces/ws-canonical", { replace: true });
    });
  });

  it("opens local sandbox recents directly into the workspace", async () => {
    vi.mocked(loadLauncherRecents).mockResolvedValueOnce([
      {
        kind: "local",
        label: "sealed-local",
        root_path: "/Users/example-user/.ctx/workspaces/staging/workspace-abc",
        execution_environment: "sandbox",
        updated_at_ms: 25,
      },
    ]);
    vi.mocked(desktopConnectLocal).mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4399",
      token: "test-token",
    } as never);
    vi.mocked(listWorkspaces).mockResolvedValue([
      {
        id: "ws-container",
        name: "Sealed Local",
        root_path: "/Users/example-user/.ctx/workspaces/staging/workspace-abc",
      },
    ] as never);

    render(<LauncherPage />);

    fireEvent.click(await screen.findByRole("button", { name: /sealed-local/i }));

    await waitFor(() => {
      expect(upsertLauncherRecent).toHaveBeenCalledWith(expect.objectContaining({
        kind: "local",
        label: "Sealed Local",
        root_path: "/Users/example-user/.ctx/workspaces/staging/workspace-abc",
        execution_environment: "sandbox",
        updated_at_ms: expect.any(Number),
      }));
      expect(navigateMock).toHaveBeenCalledWith("/workspaces/ws-container", { replace: true });
    });
  });

  it("opens remote host recents directly into the workspace after SSH connect", async () => {
    vi.mocked(loadLauncherRecents).mockResolvedValueOnce([
      {
        kind: "ssh",
        label: "remote-devbox",
        host: "devbox.example.invalid",
        user: "user",
        remote_port: 4399,
        remote_data_dir: "/tmp/ctx-daemon",
        workspace_root_path: "/srv/ctx/remote-devbox",
        execution_environment: "host",
        updated_at_ms: 40,
      },
    ]);
    vi.mocked(desktopConnectSsh).mockResolvedValue({
      kind: "ssh",
      base_url: "http://127.0.0.1:44099",
      token: "ssh-token",
      host: "devbox.example.invalid",
      user: "user",
      remote_port: 4399,
    } as never);
    vi.mocked(listWorkspaces).mockResolvedValue([
      { id: "ws-remote", name: "Remote Devbox", root_path: "/srv/ctx/remote-devbox" },
    ] as never);

    render(<LauncherPage />);

    fireEvent.click(await screen.findByRole("button", { name: /remote-devbox/i }));

    await waitFor(() => {
      expect(upsertLauncherRecent).toHaveBeenCalledWith(expect.objectContaining({
        kind: "ssh",
        label: "Remote Devbox",
        workspace_root_path: "/srv/ctx/remote-devbox",
        updated_at_ms: expect.any(Number),
      }));
      expect(navigateMock).toHaveBeenCalledWith("/workspaces/ws-remote", { replace: true });
    });
  });

  it("opens remote sandbox recents directly into the workspace after SSH connect", async () => {
    vi.mocked(loadLauncherRecents).mockResolvedValueOnce([
      {
        kind: "ssh",
        label: "sealed-remote",
        host: "sealed.example.invalid",
        user: "user",
        remote_port: 44099,
        remote_data_dir: "/tmp/ctx-remote",
        workspace_root_path: "/srv/ctx/remote-container",
        execution_environment: "sandbox",
        updated_at_ms: 30,
      },
    ]);
    vi.mocked(desktopConnectSsh).mockResolvedValue({
      kind: "ssh",
      base_url: "http://127.0.0.1:44099",
      token: "ssh-token",
      host: "sealed.example.invalid",
      user: "user",
      remote_port: 44099,
    } as never);
    vi.mocked(listWorkspaces).mockResolvedValue([
      { id: "ws-remote-container", name: "Sealed Remote", root_path: "/srv/ctx/remote-container" },
    ] as never);

    render(<LauncherPage />);

    fireEvent.click(await screen.findByRole("button", { name: /sealed-remote/i }));

    await waitFor(() => {
      expect(upsertLauncherRecent).toHaveBeenCalledWith(expect.objectContaining({
        kind: "ssh",
        label: "Sealed Remote",
        workspace_root_path: "/srv/ctx/remote-container",
        execution_environment: "sandbox",
        updated_at_ms: expect.any(Number),
      }));
      expect(navigateMock).toHaveBeenCalledWith("/workspaces/ws-remote-container", { replace: true });
    });
  });

  it("routes to wizard when a local recent path is not registered as a workspace", async () => {
    vi.mocked(loadLauncherRecents).mockResolvedValueOnce([
      {
        kind: "local",
        label: "repo-b",
        root_path: "/tmp/repo-b",
        updated_at_ms: 10,
      },
    ]);
    vi.mocked(desktopConnectLocal).mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4399",
      token: "test-token",
    } as never);
    vi.mocked(listWorkspaces).mockResolvedValue([]);

    render(<LauncherPage />);

    fireEvent.click(await screen.findByRole("button", { name: /repo-b/i }));

    await waitFor(() => {
      expect(navigateMock).toHaveBeenCalledWith("/workspace-setup");
      expect(upsertLauncherRecent).not.toHaveBeenCalled();
    });
    expect(await screen.findByText("Workspace not found for this path. Re-create it from New Workspace.")).toBeInTheDocument();
  });
});
