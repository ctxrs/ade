import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import AppSettingsPage from "./AppSettingsPage";
import { clearLauncherRecents, getLauncherRecentsCount } from "../state/launcherRecentsStore";

vi.mock("../api/client", () => ({
  resetDaemonConnection: vi.fn(),
}));

vi.mock("../api/useDaemonConnection", () => ({
  useDaemonBaseUrl: () => "http://127.0.0.1:4399",
}));

vi.mock("../utils/desktop", () => ({
  desktopDisconnect: vi.fn(),
  isDesktopApp: () => true,
}));

vi.mock("../state/launcherRecentsStore", () => ({
  getLauncherRecentsCount: vi.fn(),
  clearLauncherRecents: vi.fn(),
}));

function renderPage() {
  return render(
    <MemoryRouter>
      <AppSettingsPage />
    </MemoryRouter>,
  );
}

describe("AppSettingsPage recents", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(getLauncherRecentsCount).mockResolvedValue(3);
    vi.mocked(clearLauncherRecents).mockResolvedValue();
  });

  it("shows recents count from launcher recents store", async () => {
    renderPage();
    await waitFor(() => {
      expect(screen.getByText("3 saved entries")).toBeInTheDocument();
    });
    expect(getLauncherRecentsCount).toHaveBeenCalledTimes(1);
  });

  it("clears recents through launcher recents store", async () => {
    renderPage();
    await waitFor(() => {
      expect(screen.getByText("3 saved entries")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole("button", { name: "Clear recents" }));

    await waitFor(() => {
      expect(clearLauncherRecents).toHaveBeenCalledTimes(1);
      expect(screen.getByText("0 saved entries")).toBeInTheDocument();
    });
  });
});
