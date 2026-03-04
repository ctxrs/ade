import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import UpdateNoticeBanner from "./UpdateNoticeBanner";
import { applyAppImageUpdate, downloadAppImageUpdate } from "../api/client";
import { desktopApplyAppUpdate, desktopGetAppUpdateState, desktopRestartApp, isDesktopApp } from "../utils/desktop";
import { readCachedUpdateCheck, refreshUpdateCheck, writeCachedUpdateCheck } from "../utils/updateNotice";

vi.mock("../api/client", async (importOriginal) => {
  const original = await importOriginal<typeof import("../api/client")>();
  return {
    ...original,
    downloadAppImageUpdate: vi.fn(),
    applyAppImageUpdate: vi.fn(),
  };
});

vi.mock("../utils/desktop", async (importOriginal) => {
  const original = await importOriginal<typeof import("../utils/desktop")>();
  return {
    ...original,
    isDesktopApp: vi.fn(),
    desktopApplyAppUpdate: vi.fn(),
    desktopGetAppUpdateState: vi.fn(),
    desktopRestartApp: vi.fn(),
  };
});

vi.mock("../utils/updateNotice", () => ({
  readCachedUpdateCheck: vi.fn(),
  refreshUpdateCheck: vi.fn(),
  writeCachedUpdateCheck: vi.fn(),
}));

const PROMPT_SNOOZE_STORAGE_KEY = "ctx_update_prompt_next_allowed_at_v1";
const IDLE_UPDATE_VERSION_STORAGE_KEY = "ctx_update_prompt_idle_versions_v1";
const AUTO_APPLY_ON_LAUNCH_STORAGE_KEY = "ctx_update_auto_apply_on_launch_v1";
const RESTART_REQUIRED_VERSION_STORAGE_KEY = "ctx_update_restart_required_version_v1";

const baseUpdate = {
  channel: "stable",
  base_url: "https://example.com",
  platform: "linux-x64",
  in_place_update_supported: true,
  in_place_update_reason: null,
  current_version: "1.0.0",
  update_available: true,
} as const;

const renderBanner = (props?: { allTasksIdle?: boolean }) =>
  render(
    <MemoryRouter future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>
      <UpdateNoticeBanner {...props} />
    </MemoryRouter>,
  );

describe("UpdateNoticeBanner", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.removeItem(PROMPT_SNOOZE_STORAGE_KEY);
    window.localStorage.removeItem(IDLE_UPDATE_VERSION_STORAGE_KEY);
    window.localStorage.removeItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY);
    window.sessionStorage.removeItem(RESTART_REQUIRED_VERSION_STORAGE_KEY);
    vi.mocked(isDesktopApp).mockReturnValue(false);
    vi.mocked(downloadAppImageUpdate).mockResolvedValue({
      downloaded_path: "/tmp/ctx.AppImage.new",
      can_apply_in_place: true,
    });
    vi.mocked(applyAppImageUpdate).mockResolvedValue({
      applied: true,
      target_path: "/tmp/ctx",
      message: "ok",
    });
    vi.mocked(desktopApplyAppUpdate).mockResolvedValue({
      applied: true,
      needs_restart: true,
      latest_version: "9.9.9",
      message: "ok",
    });
    vi.mocked(readCachedUpdateCheck).mockReturnValue(null);
    vi.mocked(refreshUpdateCheck).mockResolvedValue(null);
    vi.mocked(desktopGetAppUpdateState).mockResolvedValue({
      configured: true,
      available: false,
      restart_required: false,
      current_version: "1.0.0",
      latest_version: null,
      target: "macos-arm64",
      endpoint: "https://api.example/functions/v1/releases/stable/latest-tauri.json",
      message: null,
    });
    vi.mocked(desktopRestartApp).mockResolvedValue({
      requested: true,
      message: "Restart requested.",
    });
  });

  it("renders when cached update is available", () => {
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      latest_version: "9.9.9",
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue(null);

    renderBanner({ allTasksIdle: false });
    expect(screen.getByTestId("update-available-snackbar")).toBeInTheDocument();
    expect(screen.getByText(/Update available:\s*9.9.9/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "View release notes" })).toHaveAttribute(
      "href",
      "https://ctx.rs/release-notes/9.9.9",
    );
    expect(screen.getByRole("button", { name: "Update Now" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Update on Next Idle" })).toBeInTheDocument();
  });

  it("does not render when cached update is unavailable", async () => {
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      update_available: false,
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue({
      ...baseUpdate,
      update_available: false,
    });

    const { container } = renderBanner();
    await waitFor(() => {
      expect(vi.mocked(refreshUpdateCheck)).toHaveBeenCalled();
    });
    expect(container.firstChild).toBeNull();
  });

  it("does not loop refresh checks when refresh returns equivalent data with new object identity", async () => {
    window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "0");
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      latest_version: "1.0.1",
      update_available: false,
    });
    vi.mocked(refreshUpdateCheck).mockImplementation(async () => ({
      ...baseUpdate,
      latest_version: "1.0.1",
      update_available: false,
    }));

    renderBanner({ allTasksIdle: false });
    await waitFor(() => {
      expect(vi.mocked(refreshUpdateCheck)).toHaveBeenCalledTimes(1);
    });
    await new Promise((resolve) => setTimeout(resolve, 25));
    expect(vi.mocked(refreshUpdateCheck)).toHaveBeenCalledTimes(1);
  });

  it("dismisses a version and records intent when Update on Next Idle is selected", async () => {
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      latest_version: "1.2.3",
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue(null);

    renderBanner({ allTasksIdle: false });
    await waitFor(() => {
      expect(vi.mocked(refreshUpdateCheck)).toHaveBeenCalled();
    });
    expect(screen.getByText(/Update available:\s*1.2.3/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Update on Next Idle" }));
    expect(screen.queryByText(/Update available:\s*1.2.3/)).not.toBeInTheDocument();
    expect(window.localStorage.getItem(PROMPT_SNOOZE_STORAGE_KEY)).toContain("1.2.3");
    expect(window.localStorage.getItem(IDLE_UPDATE_VERSION_STORAGE_KEY)).toContain("1.2.3");
  });

  it("auto-applies update on desktop launch and preserves restart-required state", async () => {
    window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "1");
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(readCachedUpdateCheck).mockReturnValue(null);
    vi.mocked(refreshUpdateCheck).mockResolvedValue({
      ...baseUpdate,
      latest_version: "1.2.3",
    });
    vi.mocked(desktopGetAppUpdateState).mockResolvedValue({
      configured: true,
      available: true,
      restart_required: false,
      current_version: "1.0.0",
      latest_version: "1.2.3",
      target: "macos-arm64",
      endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
      message: null,
    });

    renderBanner();
    await waitFor(() => {
      expect(vi.mocked(desktopApplyAppUpdate)).toHaveBeenCalledTimes(1);
    });
    expect(vi.mocked(downloadAppImageUpdate)).not.toHaveBeenCalled();
    expect(vi.mocked(applyAppImageUpdate)).not.toHaveBeenCalled();
    expect(screen.getByTestId("update-available-snackbar")).toBeInTheDocument();
    const restartButton = screen.getByRole("button", { name: "Restart app to finish update" });
    expect(restartButton).toBeEnabled();
    fireEvent.click(restartButton);
    await waitFor(() => {
      expect(vi.mocked(desktopRestartApp)).toHaveBeenCalledTimes(1);
    });
    expect(vi.mocked(writeCachedUpdateCheck)).not.toHaveBeenCalled();
    expect(window.sessionStorage.getItem(RESTART_REQUIRED_VERSION_STORAGE_KEY)).toBe("1.2.3");
  });

  it("falls back to native desktop updater check when daemon update check is unavailable", async () => {
    window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "1");
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(readCachedUpdateCheck).mockReturnValue(null);
    vi.mocked(refreshUpdateCheck).mockResolvedValue(null);
    vi.mocked(desktopGetAppUpdateState).mockResolvedValue({
      configured: true,
      available: true,
      restart_required: false,
      current_version: "0.4.1",
      latest_version: "0.4.3",
      target: "macos-arm64",
      endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
      message: null,
    });

    renderBanner();
    await waitFor(() => {
      expect(vi.mocked(desktopGetAppUpdateState)).toHaveBeenCalledTimes(1);
    });
    await waitFor(() => {
      expect(vi.mocked(desktopApplyAppUpdate)).toHaveBeenCalledTimes(1);
    });
    expect(screen.getByTestId("update-available-snackbar")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Restart app to finish update" })).toBeEnabled();
  });

  it("shows a visible desktop updater error notice when native check fails without prior state", async () => {
    window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "0");
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(readCachedUpdateCheck).mockReturnValue(null);
    vi.mocked(refreshUpdateCheck).mockResolvedValue(null);
    vi.mocked(desktopGetAppUpdateState).mockRejectedValue(new Error("native updater probe failed"));

    renderBanner({ allTasksIdle: false });
    await waitFor(() => {
      expect(vi.mocked(desktopGetAppUpdateState)).toHaveBeenCalledTimes(1);
    });
    expect(screen.getByTestId("update-available-snackbar")).toBeInTheDocument();
    expect(screen.getByText("Updater check failed.")).toBeInTheDocument();
    expect(screen.getByText("native updater probe failed")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry check" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Update on Next Idle" })).not.toBeInTheDocument();
  });

  it("clears desktop updater error notice after a successful retry with no update available", async () => {
    window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "0");
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(readCachedUpdateCheck).mockReturnValue(null);
    vi.mocked(refreshUpdateCheck).mockResolvedValue(null);
    vi.mocked(desktopGetAppUpdateState)
      .mockRejectedValueOnce(new Error("native updater probe failed"))
      .mockResolvedValue({
        configured: true,
        available: false,
        restart_required: false,
        current_version: "0.4.8",
        latest_version: null,
        target: "macos-arm64",
        endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
        message: null,
      });

    renderBanner({ allTasksIdle: false });
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Retry check" })).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole("button", { name: "Retry check" }));
    await waitFor(() => {
      expect(vi.mocked(desktopGetAppUpdateState)).toHaveBeenCalledTimes(2);
    });
    await waitFor(() => {
      expect(screen.queryByTestId("update-available-snackbar")).not.toBeInTheDocument();
    });
  });

  it("shows native failed-phase message from desktop updater state", async () => {
    window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "0");
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(readCachedUpdateCheck).mockReturnValue(null);
    vi.mocked(refreshUpdateCheck).mockResolvedValue(null);
    vi.mocked(desktopGetAppUpdateState).mockResolvedValue({
      configured: true,
      available: false,
      restart_required: false,
      phase: "failed",
      current_version: "0.4.14",
      latest_version: "0.4.15",
      target: "macos-arm64",
      endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
      message: "native updater download failed: Invalid symbol 32, offset 9.",
    });

    renderBanner({ allTasksIdle: false });
    await waitFor(() => {
      expect(vi.mocked(desktopGetAppUpdateState)).toHaveBeenCalledTimes(1);
    });
    expect(screen.getByTestId("update-available-snackbar")).toBeInTheDocument();
    expect(screen.getByText("Updater check failed.")).toBeInTheDocument();
    expect(screen.getByText(/Invalid symbol 32, offset 9/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry check" })).toBeInTheDocument();
  });

  it("auto-applies once staging transitions to staged-ready on desktop", async () => {
    vi.useFakeTimers();
    try {
      window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "1");
      vi.mocked(isDesktopApp).mockReturnValue(true);
      vi.mocked(readCachedUpdateCheck).mockReturnValue(null);
      vi.mocked(refreshUpdateCheck).mockResolvedValue({
        ...baseUpdate,
        current_version: "0.4.14",
        latest_version: "0.4.15",
        update_available: true,
      });
      vi.mocked(desktopGetAppUpdateState)
        .mockResolvedValueOnce({
          configured: true,
          available: false,
          restart_required: false,
          phase: "staging",
          staged: false,
          current_version: "0.4.14",
          latest_version: "0.4.15",
          target: "macos-arm64",
          endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
          message: "Downloading update in background.",
        })
        .mockResolvedValueOnce({
          configured: true,
          available: true,
          restart_required: false,
          phase: "staged_ready",
          staged: true,
          current_version: "0.4.14",
          latest_version: "0.4.15",
          target: "macos-arm64",
          endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
          message: null,
        });
      vi.mocked(desktopApplyAppUpdate).mockResolvedValue({
        applied: true,
        needs_restart: true,
        latest_version: "0.4.15",
        message: "Desktop update installed. Relaunch the app to complete the update.",
      });

      renderBanner({ allTasksIdle: false });
      await act(async () => {
        await Promise.resolve();
        await Promise.resolve();
      });
      expect(vi.mocked(desktopGetAppUpdateState)).toHaveBeenCalledTimes(1);
      expect(vi.mocked(desktopApplyAppUpdate)).not.toHaveBeenCalled();

      await act(async () => {
        await vi.advanceTimersByTimeAsync(4000);
        await Promise.resolve();
        await Promise.resolve();
      });

      await act(async () => {
        await Promise.resolve();
        await Promise.resolve();
      });
      expect(vi.mocked(desktopGetAppUpdateState)).toHaveBeenCalledTimes(2);
      expect(vi.mocked(desktopApplyAppUpdate)).toHaveBeenCalledTimes(1);
      expect(screen.getByRole("button", { name: "Restart app to finish update" })).toBeEnabled();
    } finally {
      vi.useRealTimers();
    }
  });

  it("persists restart-required state across remounts in the same app session", async () => {
    window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "0");
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      latest_version: "2.4.0",
      min_supported_version: undefined,
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue({
      ...baseUpdate,
      current_version: "1.0.0",
      latest_version: "2.4.0",
      min_supported_version: null,
    });
    vi.mocked(desktopGetAppUpdateState)
      .mockResolvedValueOnce({
        configured: true,
        available: true,
        restart_required: false,
        current_version: "1.0.0",
        latest_version: "2.4.0",
        target: "macos-arm64",
        endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
        message: null,
      })
      .mockResolvedValue({
        configured: true,
        available: false,
        restart_required: true,
        current_version: "1.0.0",
        latest_version: "2.4.0",
        target: "macos-arm64",
        endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
        message: null,
      });
    vi.mocked(desktopApplyAppUpdate).mockResolvedValue({
      applied: true,
      needs_restart: true,
      latest_version: "2.4.0",
      message: "Desktop update installed. Relaunch the app to complete the update.",
    });

    const firstRender = renderBanner({ allTasksIdle: false });
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Update Now" })).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole("button", { name: "Update Now" }));
    await waitFor(() => {
      expect(vi.mocked(desktopApplyAppUpdate)).toHaveBeenCalledTimes(1);
    });
    expect(window.sessionStorage.getItem(RESTART_REQUIRED_VERSION_STORAGE_KEY)).toBe("2.4.0");

    await act(async () => {
      firstRender.unmount();
    });
    await act(async () => {
      renderBanner({ allTasksIdle: false });
      await Promise.resolve();
    });
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Restart app to finish update" })).toBeEnabled();
    });
  });

  it("ignores stale localStorage restart marker after relaunch when offline", async () => {
    window.localStorage.setItem(RESTART_REQUIRED_VERSION_STORAGE_KEY, "2.0.0");
    window.sessionStorage.removeItem(RESTART_REQUIRED_VERSION_STORAGE_KEY);
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      latest_version: "2.0.0",
      update_available: false,
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue(null);

    const { container } = renderBanner({ allTasksIdle: false });
    await waitFor(() => {
      expect(vi.mocked(refreshUpdateCheck)).toHaveBeenCalledTimes(1);
    });
    expect(screen.queryByRole("button", { name: "Restart app to finish update" })).not.toBeInTheDocument();
    expect(container.firstChild).toBeNull();
  });

  it("forces a real check and skips launch auto-apply while restart is pending", async () => {
    window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "1");
    window.sessionStorage.setItem(RESTART_REQUIRED_VERSION_STORAGE_KEY, "2.0.0");
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      current_version: "1.0.0",
      latest_version: "2.0.0",
      min_supported_version: "1.5.0",
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue({
      ...baseUpdate,
      current_version: "1.0.0",
      latest_version: "2.0.0",
      min_supported_version: "1.5.0",
    });
    vi.mocked(desktopGetAppUpdateState).mockResolvedValue({
      configured: true,
      available: false,
      restart_required: true,
      current_version: "1.0.0",
      latest_version: "2.0.0",
      target: "macos-arm64",
      endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
      message: null,
    });

    renderBanner({ allTasksIdle: false });
    await waitFor(() => {
      expect(vi.mocked(desktopGetAppUpdateState)).toHaveBeenCalledWith("stable");
    });
    expect(vi.mocked(desktopApplyAppUpdate)).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Restart app to finish update" })).toBeEnabled();
  });

  it("clears pending restart on later refresh after initial forced check failure", async () => {
    vi.useFakeTimers();
    try {
      window.sessionStorage.setItem(RESTART_REQUIRED_VERSION_STORAGE_KEY, "2.0.0");
      vi.mocked(readCachedUpdateCheck).mockReturnValue({
        ...baseUpdate,
        current_version: "1.0.0",
        latest_version: "2.0.0",
        update_available: false,
      });
      vi.mocked(refreshUpdateCheck)
        .mockResolvedValueOnce(null)
        .mockResolvedValueOnce({
          ...baseUpdate,
          current_version: "2.0.0",
          latest_version: "2.0.0",
          update_available: false,
        });

      renderBanner({ allTasksIdle: false });
      await act(async () => {
        await Promise.resolve();
        await Promise.resolve();
      });
      expect(vi.mocked(refreshUpdateCheck)).toHaveBeenCalledWith({ force: true });
      expect(screen.getByRole("button", { name: "Restart app to finish update" })).toBeDisabled();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(60 * 60 * 1000);
        await Promise.resolve();
        await Promise.resolve();
      });
      expect(vi.mocked(refreshUpdateCheck)).toHaveBeenCalledTimes(2);
      expect(window.sessionStorage.getItem(RESTART_REQUIRED_VERSION_STORAGE_KEY)).toBeNull();
      expect(screen.queryByRole("button", { name: "Restart app to finish update" })).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("re-notifies after a 24 hour snooze window elapses", async () => {
    const nowSpy = vi.spyOn(Date, "now");
    nowSpy.mockReturnValue(1_700_000_000_000);
    window.localStorage.setItem(
      PROMPT_SNOOZE_STORAGE_KEY,
      JSON.stringify({ "2.0.0": 1_700_000_000_000 + 1_000 }),
    );
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      latest_version: "2.0.0",
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue(null);

    const { rerender } = render(
      <MemoryRouter future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>
        <UpdateNoticeBanner />
      </MemoryRouter>,
    );
    expect(screen.queryByTestId("update-available-snackbar")).not.toBeInTheDocument();

    nowSpy.mockReturnValue(1_700_000_000_000 + 24 * 60 * 60 * 1000 + 2_000);
    rerender(
      <MemoryRouter future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>
        <UpdateNoticeBanner />
      </MemoryRouter>,
    );
    expect(screen.getByTestId("update-available-snackbar")).toBeInTheDocument();
    nowSpy.mockRestore();
  });

  it("shows required-update blocker when below min supported version", () => {
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      current_version: "1.0.0",
      latest_version: "1.2.0",
      min_supported_version: "1.1.0",
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue(null);

    renderBanner();
    expect(screen.getByRole("dialog", { name: "Update required" })).toBeInTheDocument();
    expect(screen.getByText("Update required to continue")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Update Now" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Update on Next Idle" })).not.toBeInTheDocument();
  });

  it("does not force-lock when update is unavailable even if below min supported version", () => {
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      current_version: "1.0.0",
      latest_version: "1.2.0",
      min_supported_version: "1.1.0",
      update_available: false,
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue(null);

    const { container } = renderBanner();
    expect(screen.queryByRole("dialog", { name: "Update required" })).not.toBeInTheDocument();
    expect(container.firstChild).toBeNull();
  });

  it("does not force-lock when platform is unsupported", () => {
    window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "0");
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      current_version: "1.0.0",
      latest_version: "1.2.0",
      min_supported_version: "1.1.0",
      platform_supported: false,
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue(null);

    renderBanner({ allTasksIdle: false });
    expect(screen.queryByRole("dialog", { name: "Update required" })).not.toBeInTheDocument();
    expect(screen.getByTestId("update-available-snackbar")).toBeInTheDocument();
  });

  it.each([
    {
      name: "requires actionable update and stays non-blocking when update is unavailable",
      isDesktop: false,
      patch: { update_available: false },
      expectForcedDialog: false,
    },
    {
      name: "stays non-blocking when platform is unsupported",
      isDesktop: false,
      patch: { platform_supported: false },
      expectForcedDialog: false,
    },
    {
      name: "stays non-blocking when in-place capability is unavailable on non-desktop",
      isDesktop: false,
      patch: {
        in_place_update_supported: false,
        in_place_update_reason: "Install manually.",
      },
      expectForcedDialog: false,
    },
    {
      name: "blocks when non-desktop in-place capability is available",
      isDesktop: false,
      patch: { in_place_update_supported: true },
      expectForcedDialog: true,
    },
    {
      name: "blocks on desktop regardless of daemon in-place capability field",
      isDesktop: true,
      patch: { in_place_update_supported: false },
      expectForcedDialog: true,
    },
  ])("forced gate matrix: $name", async ({ isDesktop, patch, expectForcedDialog }) => {
    vi.mocked(isDesktopApp).mockReturnValue(isDesktop);
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      current_version: "1.0.0",
      latest_version: "1.2.0",
      min_supported_version: "1.1.0",
      ...patch,
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue({
      ...baseUpdate,
      current_version: "1.0.0",
      latest_version: "1.2.0",
      min_supported_version: "1.1.0",
      ...patch,
    });
    if (isDesktop) {
      vi.mocked(desktopGetAppUpdateState).mockResolvedValue({
        configured: true,
        available: true,
        restart_required: false,
        current_version: "1.0.0",
        latest_version: "1.2.0",
        target: "macos-arm64",
        endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
        message: null,
      });
    }

    renderBanner({ allTasksIdle: false });
    if (expectForcedDialog) {
      await waitFor(() => {
        expect(screen.getByRole("dialog", { name: "Update required" })).toBeInTheDocument();
      });
      return;
    }
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "Update required" })).not.toBeInTheDocument();
    });
  });

  it("treats prerelease current version as below stable min supported version", () => {
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      current_version: "1.2.3-beta.1",
      latest_version: "1.2.3",
      min_supported_version: "1.2.3",
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue({
      ...baseUpdate,
      current_version: "1.0.0",
      latest_version: "1.2.0",
      min_supported_version: "1.1.0",
    });

    renderBanner();
    expect(screen.getByRole("dialog", { name: "Update required" })).toBeInTheDocument();
  });

  it("keeps required-update blocker visible on desktop when restart is still required", async () => {
    window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "0");
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      current_version: "1.0.0",
      latest_version: "1.2.0",
      min_supported_version: "1.1.0",
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue({
      ...baseUpdate,
      current_version: "1.0.0",
      latest_version: "1.2.0",
      min_supported_version: "1.1.0",
    });
    vi.mocked(desktopGetAppUpdateState).mockResolvedValue({
      configured: true,
      available: true,
      restart_required: false,
      current_version: "1.0.0",
      latest_version: "1.2.0",
      target: "macos-arm64",
      endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
      message: null,
    });
    vi.mocked(desktopApplyAppUpdate).mockResolvedValue({
      applied: true,
      needs_restart: true,
      latest_version: "1.2.0",
      message: "Desktop update installed. Relaunch the app to complete the update.",
    });

    renderBanner();
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Update Now" })).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole("button", { name: "Update Now" }));
    await waitFor(() => {
      expect(vi.mocked(desktopApplyAppUpdate)).toHaveBeenCalledTimes(1);
    });
    expect(screen.getByRole("dialog", { name: "Update required" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Restart app to finish update" })).toBeEnabled();
  });

  it("keeps required-update blocker visible on desktop when restart is required even if apply reports false", async () => {
    window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "0");
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      current_version: "1.0.0",
      latest_version: "1.2.0",
      min_supported_version: "1.1.0",
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue({
      ...baseUpdate,
      current_version: "1.0.0",
      latest_version: "1.2.0",
      min_supported_version: "1.1.0",
    });
    vi.mocked(desktopGetAppUpdateState).mockResolvedValue({
      configured: true,
      available: true,
      restart_required: false,
      current_version: "1.0.0",
      latest_version: "1.2.0",
      target: "macos-arm64",
      endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
      message: null,
    });
    vi.mocked(desktopApplyAppUpdate).mockResolvedValue({
      applied: false,
      needs_restart: true,
      latest_version: "1.2.0",
      message: "Restart required to complete update.",
    });

    renderBanner();
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Update Now" })).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole("button", { name: "Update Now" }));
    await waitFor(() => {
      expect(vi.mocked(desktopApplyAppUpdate)).toHaveBeenCalledTimes(1);
    });
    expect(screen.getByRole("dialog", { name: "Update required" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Restart app to finish update" })).toBeEnabled();
  });

  it("keeps required-update blocker visible on appimage when relaunch is still required", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(false);
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      current_version: "1.0.0",
      latest_version: "1.2.0",
      min_supported_version: "1.1.0",
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue(null);

    renderBanner();
    fireEvent.click(screen.getByRole("button", { name: "Update Now" }));
    await waitFor(() => {
      expect(vi.mocked(downloadAppImageUpdate)).toHaveBeenCalledTimes(1);
      expect(vi.mocked(applyAppImageUpdate)).toHaveBeenCalledTimes(1);
      expect(vi.mocked(downloadAppImageUpdate).mock.invocationCallOrder[0]).toBeLessThan(
        vi.mocked(applyAppImageUpdate).mock.invocationCallOrder[0],
      );
    });
    expect(screen.getByRole("dialog", { name: "Update required" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Restart app to finish update" })).toBeDisabled();
  });

  it("applies update when Update Now is clicked", async () => {
    window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "0");
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      latest_version: "2.0.0",
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue({
      ...baseUpdate,
      latest_version: "2.0.0",
    });
    vi.mocked(desktopGetAppUpdateState)
      .mockResolvedValueOnce({
        configured: true,
        available: true,
        restart_required: false,
        current_version: "1.0.0",
        latest_version: "2.0.0",
        target: "macos-arm64",
        endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
        message: null,
      })
      .mockResolvedValue({
        configured: true,
        available: false,
        restart_required: false,
        current_version: "2.0.0",
        latest_version: null,
        target: "macos-arm64",
        endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
        message: null,
      });
    vi.mocked(desktopApplyAppUpdate).mockResolvedValue({
      applied: true,
      needs_restart: false,
      latest_version: "2.0.0",
      message: "Applied update",
    });

    renderBanner();
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Update Now" })).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole("button", { name: "Update Now" }));
    await waitFor(() => {
      expect(vi.mocked(desktopApplyAppUpdate)).toHaveBeenCalledTimes(1);
    });
    expect(vi.mocked(downloadAppImageUpdate)).not.toHaveBeenCalled();
    expect(vi.mocked(applyAppImageUpdate)).not.toHaveBeenCalled();
    await waitFor(() => {
      expect(screen.queryByTestId("update-available-snackbar")).not.toBeInTheDocument();
    });
    expect(vi.mocked(writeCachedUpdateCheck)).not.toHaveBeenCalled();
  });

  it("keeps banner visible with restart-required state after non-forced desktop apply", async () => {
    window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "0");
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      latest_version: "2.2.0",
      min_supported_version: undefined,
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue({
      ...baseUpdate,
      latest_version: "2.2.0",
      min_supported_version: null,
    });
    vi.mocked(desktopGetAppUpdateState).mockResolvedValue({
      configured: true,
      available: true,
      restart_required: false,
      current_version: "1.0.0",
      latest_version: "2.2.0",
      target: "macos-arm64",
      endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
      message: null,
    });
    vi.mocked(desktopApplyAppUpdate).mockResolvedValue({
      applied: true,
      needs_restart: true,
      latest_version: "2.2.0",
      message: "Desktop update installed. Relaunch the app to complete the update.",
    });

    renderBanner({ allTasksIdle: false });
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Update Now" })).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole("button", { name: "Update Now" }));
    await waitFor(() => {
      expect(vi.mocked(desktopApplyAppUpdate)).toHaveBeenCalledTimes(1);
    });
    expect(screen.getByTestId("update-available-snackbar")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Restart app to finish update" })).toBeEnabled();
    expect(vi.mocked(writeCachedUpdateCheck)).not.toHaveBeenCalled();
    expect(window.sessionStorage.getItem(RESTART_REQUIRED_VERSION_STORAGE_KEY)).toBe("2.2.0");
  });

  it("keeps banner visible with restart-required state after non-forced appimage apply", async () => {
    window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "0");
    vi.mocked(isDesktopApp).mockReturnValue(false);
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      latest_version: "2.3.0",
      min_supported_version: undefined,
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue(null);

    renderBanner({ allTasksIdle: false });
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Update Now" })).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole("button", { name: "Update Now" }));
    await waitFor(() => {
      expect(vi.mocked(downloadAppImageUpdate)).toHaveBeenCalledTimes(1);
      expect(vi.mocked(applyAppImageUpdate)).toHaveBeenCalledTimes(1);
    });
    expect(screen.getByTestId("update-available-snackbar")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Restart app to finish update" })).toBeDisabled();
    expect(vi.mocked(writeCachedUpdateCheck)).not.toHaveBeenCalled();
    expect(window.sessionStorage.getItem(RESTART_REQUIRED_VERSION_STORAGE_KEY)).toBe("2.3.0");
  });

  it("shows actionable error and skips appimage calls when non-desktop install does not support in-place appimage update", async () => {
    window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "0");
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      platform: "macos-arm64",
      in_place_update_supported: false,
      in_place_update_reason: "Install updates manually on this platform.",
      latest_version: "2.1.0",
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue(null);

    renderBanner({ allTasksIdle: false });
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Update Now" })).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole("button", { name: "Update Now" }));
    await waitFor(() => {
      expect(screen.getByText(/Install updates manually on this platform/i)).toBeInTheDocument();
    });
    expect(vi.mocked(downloadAppImageUpdate)).not.toHaveBeenCalled();
    expect(vi.mocked(applyAppImageUpdate)).not.toHaveBeenCalled();
  });

  it("re-shows banner when deferred idle update fails", async () => {
    window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "0");
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      latest_version: "3.0.0",
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue(null);
    vi.mocked(applyAppImageUpdate).mockRejectedValue(new Error("temporary apply failure"));

    renderBanner({ allTasksIdle: true });
    fireEvent.click(screen.getByRole("button", { name: "Update on Next Idle" }));

    await waitFor(() => {
      expect(vi.mocked(downloadAppImageUpdate)).toHaveBeenCalledTimes(1);
      expect(vi.mocked(applyAppImageUpdate)).toHaveBeenCalledTimes(1);
    });
    await waitFor(() => {
      expect(screen.getByTestId("update-available-snackbar")).toBeInTheDocument();
    });
    expect(screen.getByText(/temporary apply failure/i)).toBeInTheDocument();
    expect(window.localStorage.getItem(PROMPT_SNOOZE_STORAGE_KEY) ?? "").not.toContain("3.0.0");
    expect(window.localStorage.getItem(IDLE_UPDATE_VERSION_STORAGE_KEY) ?? "").not.toContain("3.0.0");
  });

  it("surfaces string-shaped native apply errors", async () => {
    window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "0");
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      latest_version: "3.0.1",
      platform: "macos-arm64",
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue({
      ...baseUpdate,
      latest_version: "3.0.1",
      platform: "macos-arm64",
    });
    vi.mocked(desktopGetAppUpdateState).mockResolvedValue({
      configured: true,
      available: true,
      restart_required: false,
      current_version: "1.0.0",
      latest_version: "3.0.1",
      target: "macos-arm64",
      endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
      message: null,
    });
    vi.mocked(desktopApplyAppUpdate).mockRejectedValue("native updater install failed: Permission denied");

    renderBanner({ allTasksIdle: false });
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Update Now" })).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole("button", { name: "Update Now" }));

    await waitFor(() => {
      expect(screen.getByText(/native updater install failed: Permission denied/i)).toBeInTheDocument();
    });
  });

  it("disables dismiss while update apply is in flight", async () => {
    window.localStorage.setItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY, "0");
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      latest_version: "3.1.0",
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue({
      ...baseUpdate,
      latest_version: "3.1.0",
      min_supported_version: null,
    });
    vi.mocked(desktopGetAppUpdateState).mockResolvedValue({
      configured: true,
      available: true,
      restart_required: false,
      current_version: "1.0.0",
      latest_version: "3.1.0",
      target: "macos-arm64",
      endpoint: "https://api.ctx.rs/functions/v1/releases/stable/latest-tauri.json",
      message: null,
    });
    vi.mocked(desktopApplyAppUpdate).mockImplementation(
      () =>
        new Promise(() => {
          // keep pending for in-flight UI assertion
        }),
    );

    renderBanner({ allTasksIdle: false });
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Update Now" })).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole("button", { name: "Update Now" }));
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Updating..." })).toBeDisabled();
    });
    const dismissButton = screen.getByRole("button", { name: "Dismiss update notice" });
    expect(dismissButton).toBeDisabled();
    fireEvent.click(dismissButton);
    expect(window.localStorage.getItem(PROMPT_SNOOZE_STORAGE_KEY) ?? "").not.toContain("3.1.0");
  });

  it("opens the update timing info modal from the info button", () => {
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      latest_version: "1.0.0",
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue(null);

    renderBanner();
    fireEvent.click(screen.getByRole("button", { name: "Learn about update timing" }));
    expect(screen.getByRole("dialog", { name: "Update timing info" })).toBeInTheDocument();
    expect(screen.getByText(/Updating immediately will interrupt any actively running tasks/i)).toBeInTheDocument();
  });
});
