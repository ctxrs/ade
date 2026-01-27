import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import UpdateNoticeBanner from "./UpdateNoticeBanner";
import { readCachedUpdateCheck, refreshUpdateCheck } from "../utils/updateNotice";

vi.mock("../utils/updateNotice", () => ({
  readCachedUpdateCheck: vi.fn(),
  refreshUpdateCheck: vi.fn(),
}));

const baseUpdate = {
  channel: "stable",
  base_url: "https://example.com",
  current_version: "1.0.0",
  update_available: true,
} as const;

const renderBanner = () =>
  render(
    <MemoryRouter>
      <UpdateNoticeBanner />
    </MemoryRouter>,
  );

describe("UpdateNoticeBanner", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders when cached update is available", () => {
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      latest_version: "9.9.9",
    });
    vi.mocked(refreshUpdateCheck).mockResolvedValue(null);

    renderBanner();
    expect(screen.getByText(/Update available: 9.9.9/)).toBeInTheDocument();
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

  it("forces a refresh when the user checks again", async () => {
    vi.mocked(readCachedUpdateCheck).mockReturnValue({
      ...baseUpdate,
      latest_version: "1.0.0",
    });
    vi.mocked(refreshUpdateCheck)
      .mockResolvedValueOnce({
        ...baseUpdate,
        latest_version: "1.0.0",
      })
      .mockResolvedValueOnce({
        ...baseUpdate,
        latest_version: "2.0.0",
      });

    renderBanner();
    expect(await screen.findByText(/Update available: 1.0.0/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Check again" }));
    await waitFor(() => {
      expect(vi.mocked(refreshUpdateCheck)).toHaveBeenCalledWith({ force: true });
    });
    expect(await screen.findByText(/Update available: 2.0.0/)).toBeInTheDocument();
  });
});
