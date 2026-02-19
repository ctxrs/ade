import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import DaemonAvailabilityOverlay from "./DaemonAvailabilityOverlay";
import { daemonFetchRaw } from "../api/client";
import { useDaemonBaseUrl } from "../api/useDaemonConnection";
import {
  desktopGetConnection,
  desktopGetVersion,
  desktopRestartLocalDaemon,
  desktopUpdateRemoteDaemon,
  isDesktopApp,
} from "../utils/desktop";

vi.mock("../api/client", () => ({
  applyDaemonDesktopConnection: vi.fn(),
  daemonFetchRaw: vi.fn(),
}));

vi.mock("../api/useDaemonConnection", () => ({
  useDaemonBaseUrl: vi.fn(),
}));

vi.mock("../utils/desktop", () => ({
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
    expect(screen.getByRole("link", { name: "Open diagnostics" })).toBeInTheDocument();
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
});
