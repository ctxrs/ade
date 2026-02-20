import React from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TitleGenerationInstallBanner } from "./TitleGenerationInstallBanner";
import {
  getInstall,
  getSettings,
  getTitleGenerationLocalStatus,
  type Settings,
  type TitleGenerationLocalStatus,
} from "../api/client";

vi.mock("../api/client", () => ({
  getInstall: vi.fn(),
  getSettings: vi.fn(),
  getTitleGenerationLocalStatus: vi.fn(),
}));

const buildLocalModeSettings = (): Settings => ({
  title_generation: {
    mode: "local",
    remote: {
      base_url: "https://example.test/v1",
      api_key: "",
      model: "test/model",
      use_json: true,
    },
    local: {
      model_id: "ggml-org/Qwen3-1.7B-GGUF",
      use_json: true,
    },
  },
});

const buildLocalStatus = (overrides?: Partial<TitleGenerationLocalStatus>): TitleGenerationLocalStatus => ({
  ready: false,
  runtime: {
    version: "1.0.0",
    installed: true,
    path: "/tmp/runtime",
  },
  model: {
    model_id: "ggml-org/Qwen3-1.7B-GGUF",
    file_name: "model.gguf",
    installed: false,
    version: null,
    sha256: null,
    size_bytes: null,
    installed_at: null,
  },
  install_id: "install-1",
  install_running: true,
  ...overrides,
});

describe("TitleGenerationInstallBanner", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
  });

  it("renders running local titling download progress", async () => {
    vi.mocked(getSettings).mockResolvedValue(buildLocalModeSettings());
    vi.mocked(getTitleGenerationLocalStatus).mockResolvedValue(buildLocalStatus());
    vi.mocked(getInstall).mockResolvedValue({
      install_id: "install-1",
      provider_id: "title_generation_local",
      state: "running",
      started_at: "2026-02-20T00:00:00Z",
      last_event: {
        install_id: "install-1",
        provider_id: "title_generation_local",
        at: "2026-02-20T00:00:01Z",
        stage: "download_model",
        message: "Downloading model file",
        level: "info",
        bytes: 4,
        total_bytes: 10,
      },
    });

    render(<TitleGenerationInstallBanner />);

    expect(await screen.findByText("Session titling model download in progress.")).toBeInTheDocument();
    expect(await screen.findByText("Downloading… 40%")).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "Session titling install progress" })).toHaveAttribute("aria-valuenow", "40");
  });

  it("persists dismissal per install id across remount", async () => {
    vi.mocked(getSettings).mockResolvedValue(buildLocalModeSettings());
    vi.mocked(getTitleGenerationLocalStatus).mockResolvedValue(buildLocalStatus());
    vi.mocked(getInstall).mockResolvedValue({
      install_id: "install-1",
      provider_id: "title_generation_local",
      state: "running",
      started_at: "2026-02-20T00:00:00Z",
      last_event: {
        install_id: "install-1",
        provider_id: "title_generation_local",
        at: "2026-02-20T00:00:01Z",
        stage: "download_model",
        message: "Downloading model file",
        level: "info",
        bytes: 4,
        total_bytes: 10,
      },
    });

    const rendered = render(<TitleGenerationInstallBanner />);
    expect(await screen.findByText("Session titling model download in progress.")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    await waitFor(() => {
      expect(screen.queryByText("Session titling model download in progress.")).not.toBeInTheDocument();
    });

    rendered.unmount();
    render(<TitleGenerationInstallBanner />);

    await waitFor(() => {
      expect(screen.queryByText("Session titling model download in progress.")).not.toBeInTheDocument();
    });
  });
});
