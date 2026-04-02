import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import DaemonAvailabilityOverlay from "./DaemonAvailabilityOverlay";
import { daemonFetchRaw, listWorkspaceTasks, listWorkspaces } from "../api/client";
import { useDaemonBaseUrl } from "../api/useDaemonConnection";
import {
  desktopApplyAppUpdate,
  desktopGetConnection,
  desktopGetVersion,
  desktopRestartLocalDaemon,
  desktopUpdateRemoteDaemon,
  isDesktopApp,
} from "../utils/desktop";

vi.mock("../api/client", () => ({
  applyDaemonDesktopConnection: vi.fn(),
  daemonFetchRaw: vi.fn(),
  listWorkspaces: vi.fn(),
  listWorkspaceTasks: vi.fn(),
}));

vi.mock("../api/useDaemonConnection", () => ({
  useDaemonBaseUrl: vi.fn(),
}));

vi.mock("../utils/desktop", () => ({
  desktopApplyAppUpdate: vi.fn(),
  desktopConnectLocal: vi.fn(),
  desktopGetConnection: vi.fn(),
  desktopGetVersion: vi.fn(),
  desktopRestartLocalDaemon: vi.fn(),
  desktopUpdateRemoteDaemon: vi.fn(),
  isDesktopApp: vi.fn(),
}));

const renderOverlay = (path = "/workspaces/ws-1") =>
  render(
    <MemoryRouter
      initialEntries={[path]}
      future={{ v7_startTransition: true, v7_relativeSplatPath: true }}
    >
      <DaemonAvailabilityOverlay />
    </MemoryRouter>,
  );

const baseHealth = {
  version: "1.0.0",
  daemon_version: "1.0.0",
  pid: 123,
  data_root: "/tmp",
  daemon_url: "http://127.0.0.1:4399",
  auth_required: false,
  compatibility: {
    desktop_exact_version: "1.0.0",
    mobile_api_min: 1,
    mobile_api_max: 1,
  },
};

describe("DaemonAvailabilityOverlay", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(useDaemonBaseUrl).mockReturnValue(null);
    vi.mocked(desktopGetConnection).mockResolvedValue({ kind: "local" });
    vi.mocked(desktopRestartLocalDaemon).mockResolvedValue({ kind: "local" });
    vi.mocked(desktopUpdateRemoteDaemon).mockResolvedValue({ updated: true, message: "ok" });
    vi.mocked(desktopApplyAppUpdate).mockResolvedValue({
      applied: true,
      needs_restart: true,
      latest_version: "2.0.0",
      message: "updated",
    });
    vi.mocked(listWorkspaces).mockResolvedValue([]);
    vi.mocked(listWorkspaceTasks).mockResolvedValue([]);
    vi.spyOn(window, "confirm").mockReturnValue(true);
  });

  it("shows mismatch when the daemon is older than the desktop app", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(desktopGetVersion).mockResolvedValue("2.0.0");
    vi.mocked(daemonFetchRaw).mockResolvedValue({
      status: 200,
      body: JSON.stringify({
        ...baseHealth,
        daemon_version: "1.0.0",
        compatibility: { ...baseHealth.compatibility, desktop_exact_version: "1.0.0" },
      }),
      content_type: "application/json",
    });

    renderOverlay();
    expect(await screen.findByText("Desktop and daemon are out of sync")).toBeInTheDocument();
    expect(screen.getByText(/local daemon is older than this desktop app/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Restart local daemon" })).toBeInTheDocument();
  });

  it("shows mismatch when the desktop app is older than the daemon", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(desktopGetVersion).mockResolvedValue("1.0.0");
    vi.mocked(daemonFetchRaw).mockResolvedValue({
      status: 200,
      body: JSON.stringify({
        ...baseHealth,
        daemon_version: "2.0.0",
        compatibility: { ...baseHealth.compatibility, desktop_exact_version: "2.0.0" },
      }),
      content_type: "application/json",
    });

    renderOverlay();
    expect(await screen.findByText("Desktop and daemon are out of sync")).toBeInTheDocument();
    expect(screen.getByText(/desktop app is older than the daemon/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Update desktop app" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Open diagnostics" })).toBeInTheDocument();
  });

  it("can trigger in-place desktop update from desktop-older mismatch", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(desktopGetVersion).mockResolvedValue("1.0.0");
    vi.mocked(daemonFetchRaw).mockResolvedValue({
      status: 200,
      body: JSON.stringify({
        ...baseHealth,
        daemon_version: "2.0.0",
        compatibility: { ...baseHealth.compatibility, desktop_exact_version: "2.0.0" },
      }),
      content_type: "application/json",
    });

    renderOverlay();
    expect(await screen.findByRole("button", { name: "Update desktop app" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Update desktop app" }));
    await waitFor(() => {
      expect(vi.mocked(desktopApplyAppUpdate)).toHaveBeenCalledWith("stable");
    });
    expect(
      await screen.findByText(/restart the desktop app to apply the update/i),
    ).toBeInTheDocument();
  });

  it("shows SSH-specific update action for daemon older mismatch", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(desktopGetConnection).mockResolvedValue({ kind: "ssh" });
    vi.mocked(desktopGetVersion).mockResolvedValue("2.0.0");
    vi.mocked(daemonFetchRaw).mockResolvedValue({
      status: 200,
      body: JSON.stringify({
        ...baseHealth,
        daemon_version: "1.0.0",
        compatibility: { ...baseHealth.compatibility, desktop_exact_version: "1.0.0" },
      }),
      content_type: "application/json",
    });

    renderOverlay();
    expect(await screen.findByRole("button", { name: "Update remote daemon" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Update remote daemon" }));
    await waitFor(() => {
      expect(vi.mocked(desktopUpdateRemoteDaemon)).toHaveBeenCalledWith("stable");
    });
  });

  it("prompts before remote daemon update when running tasks are active", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(desktopGetConnection).mockResolvedValue({ kind: "ssh" });
    vi.mocked(desktopGetVersion).mockResolvedValue("2.0.0");
    vi.mocked(listWorkspaces).mockResolvedValue([
      {
        id: "ws-1",
        name: "ws-1",
        root_path: "/tmp/ws-1",
        created_at: "2026-02-20T00:00:00Z",
      },
    ]);
    vi.mocked(listWorkspaceTasks).mockResolvedValue([
      {
        id: "task-1",
        workspace_id: "ws-1",
        title: "task-1",
        status: "running",
        created_at: "2026-02-20T00:00:00Z",
        updated_at: "2026-02-20T00:00:00Z",
      },
    ]);
    vi.spyOn(window, "confirm").mockReturnValue(false);
    vi.mocked(daemonFetchRaw).mockResolvedValue({
      status: 200,
      body: JSON.stringify({
        ...baseHealth,
        daemon_version: "1.0.0",
        compatibility: { ...baseHealth.compatibility, desktop_exact_version: "1.0.0" },
      }),
      content_type: "application/json",
    });

    renderOverlay();
    expect(await screen.findByRole("button", { name: "Update remote daemon" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Update remote daemon" }));
    await waitFor(() => {
      expect(vi.mocked(desktopUpdateRemoteDaemon)).not.toHaveBeenCalled();
    });
  });

  it("runs remote daemon update as a single flight across rapid clicks", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(desktopGetConnection).mockResolvedValue({ kind: "ssh" });
    vi.mocked(desktopGetVersion).mockResolvedValue("2.0.0");
    vi.mocked(daemonFetchRaw).mockResolvedValue({
      status: 200,
      body: JSON.stringify({
        ...baseHealth,
        daemon_version: "1.0.0",
        compatibility: { ...baseHealth.compatibility, desktop_exact_version: "1.0.0" },
      }),
      content_type: "application/json",
    });
    let releaseGate!: () => void;
    const precheckGate = new Promise<void>((resolve) => {
      releaseGate = () => resolve();
    });
    vi.mocked(listWorkspaces).mockImplementation(async () => {
      await precheckGate;
      return [];
    });

    renderOverlay();
    const button = await screen.findByRole("button", { name: "Update remote daemon" });
    fireEvent.click(button);
    fireEvent.click(button);
    expect(vi.mocked(listWorkspaces)).toHaveBeenCalledTimes(1);
    releaseGate();

    await waitFor(() => {
      expect(vi.mocked(desktopUpdateRemoteDaemon)).toHaveBeenCalledTimes(1);
    });
  });

  it("runs local daemon restart as a single flight across rapid clicks", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(desktopGetConnection).mockResolvedValue({ kind: "local" });
    vi.mocked(desktopGetVersion).mockResolvedValue("2.0.0");
    vi.mocked(daemonFetchRaw).mockResolvedValue({
      status: 200,
      body: JSON.stringify({
        ...baseHealth,
        daemon_version: "1.0.0",
        compatibility: { ...baseHealth.compatibility, desktop_exact_version: "1.0.0" },
      }),
      content_type: "application/json",
    });
    let releaseGate!: () => void;
    const restartGate = new Promise<void>((resolve) => {
      releaseGate = () => resolve();
    });
    vi.mocked(desktopRestartLocalDaemon).mockImplementation(async () => {
      await restartGate;
      return { kind: "local" };
    });

    renderOverlay();
    const button = await screen.findByRole("button", { name: "Restart local daemon" });
    fireEvent.click(button);
    fireEvent.click(button);
    expect(vi.mocked(desktopRestartLocalDaemon)).toHaveBeenCalledTimes(1);
    releaseGate();

    await waitFor(() => {
      expect(vi.mocked(desktopRestartLocalDaemon)).toHaveBeenCalledTimes(1);
    });
  });

  it("normalizes desktop version prefixes before comparing", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(desktopGetVersion).mockResolvedValue("v1.2.3");
    vi.mocked(daemonFetchRaw).mockResolvedValue({
      status: 200,
      body: JSON.stringify({
        ...baseHealth,
        daemon_version: "1.2.3",
        compatibility: { ...baseHealth.compatibility, desktop_exact_version: "1.2.3" },
      }),
      content_type: "application/json",
    });

    const { container } = renderOverlay();
    await waitFor(() => {
      expect(vi.mocked(daemonFetchRaw)).toHaveBeenCalled();
    });
    expect(container.firstChild).toBeNull();
  });

  it("uses non-desktop copy when running in the browser", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(false);
    vi.mocked(daemonFetchRaw).mockResolvedValue({
      status: 503,
      body: JSON.stringify({ error: "nope" }),
      content_type: "application/json",
    });

    renderOverlay();
    expect(await screen.findByText("ctx daemon unavailable")).toBeInTheDocument();
    expect(
      screen.getByText("The daemon is not reachable. Start it, then retry this screen."),
    ).toBeInTheDocument();
    expect(screen.queryByText("Open launcher")).not.toBeInTheDocument();
  });

  it("does not poll daemon availability while the workspace setup route suppresses the overlay", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(desktopGetVersion).mockResolvedValue("2.0.0");
    vi.mocked(daemonFetchRaw).mockResolvedValue({
      status: 200,
      body: JSON.stringify(baseHealth),
      content_type: "application/json",
    });

    renderOverlay("/workspace-setup");
    await new Promise((resolve) => window.setTimeout(resolve, 25));

    expect(vi.mocked(daemonFetchRaw)).not.toHaveBeenCalled();
    expect(vi.mocked(desktopGetConnection)).not.toHaveBeenCalled();
    expect(screen.queryByText("ctx daemon unavailable")).not.toBeInTheDocument();
  });
});
