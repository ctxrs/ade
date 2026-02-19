import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import DiagnosticsPage from "./DiagnosticsPage";
import {
  applyDaemonDesktopConnection,
  appendDesktopLog,
  checkUpdates,
  getDiagnostics,
  getLspStatus,
} from "../api/client";
import type { Diagnostics, LspStatus } from "../api/client";
import {
  desktopApplyAppUpdate,
  desktopCheckAppUpdate,
  desktopGetConnection,
  desktopRestartLocalDaemon,
  getDesktopPlatform,
  isDesktopApp,
  openExternalLink,
} from "../utils/desktop";

vi.mock("../api/client", () => ({
  applyDaemonDesktopConnection: vi.fn(),
  appendDesktopLog: vi.fn(),
  applyAppImageUpdate: vi.fn(),
  checkUpdates: vi.fn(),
  downloadAppImageUpdate: vi.fn(),
  getDiagnostics: vi.fn(),
  getLspStatus: vi.fn(),
  openLogsFolder: vi.fn(),
}));

vi.mock("../utils/desktop", () => ({
  desktopApplyAppUpdate: vi.fn(),
  desktopCheckAppUpdate: vi.fn(),
  desktopGetConnection: vi.fn(),
  desktopRestartLocalDaemon: vi.fn(),
  desktopUpdateRemoteDaemon: vi.fn(),
  getDesktopPlatform: vi.fn(),
  isDesktopApp: vi.fn(),
  openExternalLink: vi.fn(),
}));

const renderPage = () =>
  render(
    <MemoryRouter
      initialEntries={["/diagnostics"]}
      future={{ v7_startTransition: true, v7_relativeSplatPath: true }}
    >
      <DiagnosticsPage />
    </MemoryRouter>,
  );

describe("DiagnosticsPage updates", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(appendDesktopLog).mockResolvedValue(undefined);
    const diagnostics: Diagnostics = {
      daemon: {
        version: "1.0.0",
        daemon_version: "1.0.0",
        pid: 123,
        daemon_url: "http://127.0.0.1:4399",
        data_root: "/tmp/ctx",
        auth_required: false,
        compatibility: {
          desktop_exact_version: "1.0.0",
          mobile_api_min: 1,
          mobile_api_max: 1,
        },
      },
      platform: { os: "linux", arch: "x86_64" },
      logs: { dir: "/tmp/ctx/logs", files: [] },
      providers: [],
      managed_installs: {},
    };
    const lspStatus: LspStatus = {
      enabled: true,
      edit_plans_enabled: true,
      servers: [],
    };
    vi.mocked(getDiagnostics).mockResolvedValue(diagnostics);
    vi.mocked(getLspStatus).mockResolvedValue(lspStatus);
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(desktopGetConnection).mockResolvedValue({ kind: "local" });
    vi.mocked(getDesktopPlatform).mockResolvedValue("windows");
    vi.mocked(openExternalLink).mockResolvedValue(true);
    vi.mocked(desktopCheckAppUpdate).mockResolvedValue({
      configured: false,
      available: false,
      current_version: "1.0.0",
      latest_version: null,
      target: "windows-x64",
      endpoint: "https://api.example/functions/v1/releases/stable/latest-tauri.json",
      message: "Native updater is not configured (missing CTX_DESKTOP_UPDATER_PUBKEY).",
    });
    vi.mocked(desktopApplyAppUpdate).mockResolvedValue({
      applied: true,
      needs_restart: true,
      latest_version: "1.0.1",
      message: "Desktop update installed. Relaunch the app to complete the update.",
    });
  });

  it("opens desktop artifact URL for non-linux platforms", async () => {
    vi.mocked(checkUpdates).mockResolvedValue({
      channel: "stable",
      base_url: "https://api.example/functions/v1",
      platform: "windows-x64",
      current_version: "1.0.0",
      latest_version: "1.0.1",
      update_available: true,
      platform_supported: true,
      manifest: {
        platforms: {
          "windows-x64": {
            nsis: {
              url_path: "/download/stable/1.0.1/ctx_1.0.1_windows-x64.exe",
              sha256: "abc",
            },
          },
        },
      },
    });

    renderPage();
    fireEvent.click(await screen.findByRole("button", { name: "Check updates" }));

    const openButton = await screen.findByRole("button", { name: "Open latest desktop download" });
    fireEvent.click(openButton);

    await waitFor(() => {
      expect(openExternalLink).toHaveBeenCalledWith(
        "https://api.example/functions/v1/download/stable/1.0.1/ctx_1.0.1_windows-x64.exe",
      );
    });
  });

  it("shows unsupported-platform notice when no desktop artifact exists", async () => {
    vi.mocked(checkUpdates).mockResolvedValue({
      channel: "stable",
      base_url: "https://api.example/functions/v1",
      platform: "windows-x64",
      current_version: "1.0.0",
      latest_version: "1.0.1",
      update_available: false,
      platform_supported: false,
      manifest: { platforms: {} },
    });

    renderPage();
    fireEvent.click(await screen.findByRole("button", { name: "Check updates" }));

    expect(
      await screen.findByText(
        "Update metadata found, but this platform currently has no desktop artifact.",
      ),
    ).toBeInTheDocument();
  });

  it("shows Linux AppImage controls when the platform is linux", async () => {
    vi.mocked(checkUpdates).mockResolvedValue({
      channel: "stable",
      base_url: "https://api.example/functions/v1",
      platform: "linux-x64",
      current_version: "1.0.0",
      latest_version: "1.0.1",
      update_available: true,
      platform_supported: true,
      manifest: {
        platforms: {
          "linux-x64": {
            appimage: {
              url_path: "/download/stable/1.0.1/ctx_1.0.1_amd64.AppImage",
              sha256: "abc",
            },
          },
        },
      },
    });

    renderPage();
    fireEvent.click(await screen.findByRole("button", { name: "Check updates" }));

    expect(await screen.findByRole("button", { name: "Download AppImage update" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Open latest desktop download" })).not.toBeInTheDocument();
  });

  it("uses native updater action when configured for non-linux platforms", async () => {
    vi.mocked(desktopCheckAppUpdate).mockResolvedValue({
      configured: true,
      available: true,
      current_version: "1.0.0",
      latest_version: "1.0.1",
      target: "windows-x64",
      endpoint: "https://api.example/functions/v1/releases/stable/latest-tauri.json",
      message: null,
    });
    vi.mocked(checkUpdates).mockResolvedValue({
      channel: "stable",
      base_url: "https://api.example/functions/v1",
      platform: "windows-x64",
      current_version: "1.0.0",
      latest_version: "1.0.1",
      update_available: true,
      platform_supported: true,
      manifest: {
        platforms: {
          "windows-x64": {
            nsis: {
              url_path: "/download/stable/1.0.1/ctx_1.0.1_windows-x64.exe",
              sha256: "abc",
            },
          },
        },
      },
    });

    renderPage();
    fireEvent.click(await screen.findByRole("button", { name: "Check updates" }));

    const installButton = await screen.findByRole("button", { name: "Install desktop update" });
    fireEvent.click(installButton);

    await waitFor(() => {
      expect(desktopApplyAppUpdate).toHaveBeenCalledWith("stable");
    });
  });

  it("reapplies daemon client config after local daemon restart", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    vi.mocked(desktopGetConnection)
      .mockResolvedValueOnce({ kind: "local", base_url: "http://127.0.0.1:4399", token: "tok-old" })
      .mockResolvedValueOnce({ kind: "local", base_url: "http://127.0.0.1:4401", token: "tok-new" });
    vi.mocked(desktopRestartLocalDaemon).mockResolvedValue(undefined);
    vi.mocked(checkUpdates).mockResolvedValue({
      channel: "stable",
      base_url: "https://api.example/functions/v1",
      platform: "windows-x64",
      current_version: "1.0.0",
      latest_version: "1.0.1",
      update_available: false,
      platform_supported: true,
      manifest: { platforms: {} },
    });

    renderPage();
    const restartButton = await screen.findByRole("button", { name: "Restart local daemon" });
    fireEvent.click(restartButton);

    await waitFor(() => {
      expect(desktopRestartLocalDaemon).toHaveBeenCalledTimes(1);
      expect(applyDaemonDesktopConnection).toHaveBeenCalledWith({
        kind: "local",
        base_url: "http://127.0.0.1:4401",
        token: "tok-new",
      });
    });
  });
});
