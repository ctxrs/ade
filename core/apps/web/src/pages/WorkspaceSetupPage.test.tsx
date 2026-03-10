import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import WorkspaceSetupPage from "./WorkspaceSetupPage";
import {
  buildExecutionLaunchWsUrl,
  cancelInstall,
  createWorkspace,
  getInstall,
  getExecutionLaunchStatus,
  getHealth,
  getSettings,
  installProvider,
  listProviders,
  getTitleGenerationLocalStatus,
  importProviderAuthCandidates,
  installTitleGenerationLocal,
  listProviderAuthImportCandidates,
  listWorkspaces,
  repoClone,
  repoInit,
  repoStagingPath,
  repoStatus,
  repoValidateDestination,
  startWorkspaceSetupLaunchHandoff,
  updateSettings,
  updateWorkspaceExecutionConfig,
  updateWorkspaceMergeQueueConfig,
  updateWorkspaceWorktreeBootstrapConfig,
} from "../api/client";
import {
  desktopConnectLocal,
  desktopConnectSsh,
  desktopKickoffRemotePrewarm,
  desktopListSshHosts,
  desktopTestSsh,
  isDesktopApp,
} from "../utils/desktop";
import { upsertLauncherRecent } from "../state/launcherRecentsStore";
import { clearInstallProgress } from "../state/installProgressMonitor";
import { clearProviderInstallProgress } from "../state/providerInstallProgressStore";

const {
  trackWizardStartedMock,
  trackWizardStepViewedMock,
  trackWizardStepCompletedMock,
  trackWizardCompletedMock,
  trackWizardAbandonedMock,
  trackWorkspaceLaunchCompletedMock,
} = vi.hoisted(() => ({
  trackWizardStartedMock: vi.fn(),
  trackWizardStepViewedMock: vi.fn(),
  trackWizardStepCompletedMock: vi.fn(),
  trackWizardCompletedMock: vi.fn(),
  trackWizardAbandonedMock: vi.fn(),
  trackWorkspaceLaunchCompletedMock: vi.fn(),
}));
const getInstallMock = vi.hoisted(() => vi.fn());

vi.mock("../api/client", async () => {
  const actual = await vi.importActual<typeof import("../api/client")>("../api/client");
  return {
    ...actual,
    applyDaemonDesktopConnection: vi.fn(),
    buildExecutionLaunchWsUrl: vi.fn(),
    cancelInstall: vi.fn(),
    createWorkspace: vi.fn(),
    getInstall: getInstallMock,
    getInstallStatuses: vi.fn(async (installIds: string[]) => ({
      installs: await Promise.all(
        installIds.map(async (installId) => {
          const info = await getInstallMock(installId);
          return {
            install_id: installId,
            info: info && typeof info.install_id === "string" ? info : null,
          };
        }),
      ),
    })),
    getExecutionLaunchStatus: vi.fn(),
    getHealth: vi.fn(),
    getSettings: vi.fn(),
    installProvider: vi.fn(),
    listProviders: vi.fn(),
    getTitleGenerationLocalStatus: vi.fn(),
    importProviderAuthCandidates: vi.fn(),
    installTitleGenerationLocal: vi.fn(),
    listInstallEvents: vi.fn(async (installId: string) => {
      const info = await getInstallMock(installId);
      return info?.last_event ? [info.last_event] : [];
    }),
    listProviderAuthImportCandidates: vi.fn(),
    listWorkspaces: vi.fn(),
    repoClone: vi.fn(),
    repoInit: vi.fn(),
    repoStatus: vi.fn(),
    repoValidateDestination: vi.fn(),
    repoStagingPath: vi.fn(),
    startWorkspaceSetupLaunchHandoff: vi.fn(),
    updateSettings: vi.fn(),
    updateWorkspaceExecutionConfig: vi.fn(),
    updateWorkspaceMergeQueueConfig: vi.fn(),
    updateWorkspaceWorktreeBootstrapConfig: vi.fn(),
  };
});

vi.mock("../utils/analytics", async () => {
  const actual = await vi.importActual<typeof import("../utils/analytics")>("../utils/analytics");
  return {
    ...actual,
    trackWizardStarted: trackWizardStartedMock,
    trackWizardStepViewed: trackWizardStepViewedMock,
    trackWizardStepCompleted: trackWizardStepCompletedMock,
    trackWizardCompleted: trackWizardCompletedMock,
    trackWizardAbandoned: trackWizardAbandonedMock,
    trackWorkspaceLaunchCompleted: trackWorkspaceLaunchCompletedMock,
  };
});

vi.mock("../utils/desktop", async () => {
  const actual = await vi.importActual<typeof import("../utils/desktop")>("../utils/desktop");
  return {
    ...actual,
    desktopConnectLocal: vi.fn(),
    desktopConnectSsh: vi.fn(),
    desktopKickoffRemotePrewarm: vi.fn(),
    desktopListSshHosts: vi.fn(),
    desktopListSshPaths: vi.fn(),
    desktopGetGitBranch: vi.fn(),
    desktopPickFolder: vi.fn(),
    desktopTestSsh: vi.fn(),
    isDesktopApp: vi.fn(),
  };
});

vi.mock("../state/launcherRecentsStore", () => ({
  upsertLauncherRecent: vi.fn(),
}));

const renderPage = () =>
  render(
    <MemoryRouter initialEntries={["/workspace-setup"]}>
      <WorkspaceSetupPage />
    </MemoryRouter>,
  );

const getWizardShell = (): HTMLElement => screen.getByTestId("workspace-setup");

const wizardStepKey = (): string => getWizardShell().getAttribute("data-step-key") ?? "";

const captureStepSequence = () => {
  const seen = [wizardStepKey()];
  const observer = new MutationObserver(() => {
    const next = wizardStepKey();
    if (seen[seen.length - 1] !== next) {
      seen.push(next);
    }
  });
  observer.observe(getWizardShell(), {
    attributes: true,
    attributeFilter: ["data-step-key"],
  });
  return {
    seen,
    disconnect: () => observer.disconnect(),
  };
};

const selectLocalAndContinue = async () => {
  fireEvent.click(screen.getByTestId("wizard-option-location-local"));
  await waitFor(() => {
    expect(wizardStepKey()).not.toBe("location");
  });
};

const advancePastContainerForHost = async () => {
  await waitFor(() => {
    expect(wizardStepKey()).toBe("container");
  });
  fireEvent.click(screen.getByTestId("wizard-option-container-no-container"));
  await waitFor(() => {
    expect(wizardStepKey()).not.toBe("container");
  });
  if (wizardStepKey() === "harness-downloads") {
    fireEvent.click(screen.getByTestId("wizard-harness-skip"));
  }
};

const advanceToTitlingStep = async () => {
  await advancePastContainerForHost();
  if (wizardStepKey() === "auth-import") {
    fireEvent.click(screen.getByRole("button", { name: "Skip for now" }));
  }
  await waitFor(() => {
    expect(wizardStepKey()).toBe("session-titling");
  });
};

const providerStatusFixture = (overrides?: Partial<import("@ctx/types").ProviderStatus>): import("@ctx/types").ProviderStatus => ({
  provider_id: "codex",
  installed: false,
  health: "error",
  diagnostics: [],
  details: {
    install_supported: "true",
  },
  ...overrides,
});

const configuredTitlingSettingsFixture = () => ({
  title_generation: {
    mode: "remote",
    remote: {
      base_url: "https://openrouter.ai/api/v1",
      api_key: "sk-ready",
      model: "google/gemini-3-flash-preview",
      use_json: true,
    },
    local: {
      model_id: "ggml-org/Qwen3-1.7B-GGUF",
      use_json: true,
    },
  },
});

describe("WorkspaceSetupPage", () => {
  afterEach(() => {
    clearInstallProgress();
    clearProviderInstallProgress();
    vi.useRealTimers();
  });

  beforeEach(() => {
    vi.clearAllMocks();
    clearInstallProgress();
    clearProviderInstallProgress();
    window.localStorage.clear();
    getInstallMock.mockReset();
    trackWizardStartedMock.mockReset();
    trackWizardStepViewedMock.mockReset();
    trackWizardStepCompletedMock.mockReset();
    trackWizardCompletedMock.mockReset();
    trackWizardAbandonedMock.mockReset();
    trackWorkspaceLaunchCompletedMock.mockReset();
    vi.mocked(buildExecutionLaunchWsUrl).mockReturnValue("ws://127.0.0.1:1/launch");
    vi.mocked(isDesktopApp).mockReturnValue(false);
    vi.mocked(listProviderAuthImportCandidates).mockResolvedValue({ candidates: [] });
    vi.mocked(listWorkspaces).mockResolvedValue([]);
    vi.mocked(getHealth).mockResolvedValue({
      daemon_version: "0.0.0-test",
      compatibility: { desktop_exact_version: "0.0.0-test", mobile_api_min: 1, mobile_api_max: 1 },
    } as never);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);
    vi.mocked(listProviders).mockResolvedValue([] as never);
    vi.mocked(installProvider).mockResolvedValue({ provider_id: "codex", install_id: "install_provider_test" } as never);
    vi.mocked(getTitleGenerationLocalStatus).mockResolvedValue({
      ready: false,
      runtime: { version: "0.0.0-test", installed: false, path: null },
      model: {
        model_id: "ggml-org/Qwen3-1.7B-GGUF",
        file_name: "Qwen3-1.7B-GGUF.gguf",
        installed: false,
        version: null,
        sha256: null,
        size_bytes: null,
        installed_at: null,
      },
      install_id: null,
      install_running: false,
    } as never);
    vi.mocked(updateSettings).mockResolvedValue({} as never);
    vi.mocked(installTitleGenerationLocal).mockResolvedValue({ install_id: "install_test" } as never);
    vi.mocked(getInstall).mockResolvedValue({
      install_id: "install_test",
      provider_id: "title_generation_local",
      state: "succeeded",
      started_at: "2026-02-18T00:00:00Z",
      finished_at: "2026-02-18T00:00:01Z",
      error: undefined,
      last_event: undefined,
    } as never);
    vi.mocked(repoStatus).mockResolvedValue({ canonical_path: "/tmp/repo", is_repo: true });
    vi.mocked(repoValidateDestination).mockResolvedValue({ path: "/tmp/repo" });
    vi.mocked(repoInit).mockResolvedValue({ path: "/tmp/repo" });
    vi.mocked(repoStagingPath).mockResolvedValue({ path: "/tmp/staging" } as never);
    vi.mocked(repoClone).mockResolvedValue({ path: "/tmp/staging/repo" });
    vi.mocked(createWorkspace).mockResolvedValue({ id: "ws_test", root_path: "/tmp/ws" } as never);
    vi.mocked(updateWorkspaceExecutionConfig).mockResolvedValue({ ok: true } as never);
    vi.mocked(updateWorkspaceMergeQueueConfig).mockResolvedValue({ ok: true } as never);
    vi.mocked(updateWorkspaceWorktreeBootstrapConfig).mockResolvedValue({ ok: true } as never);
    vi.mocked(startWorkspaceSetupLaunchHandoff).mockResolvedValue({
      job_id: "job_test",
      workspace_id: "ws_test",
      kind: "workspace_launch",
      state: "ready",
      current_phase: "ready",
      phases: [],
      logs: [],
      error: null,
    } as never);
    vi.mocked(getExecutionLaunchStatus).mockResolvedValue({
      job_id: "job_test",
      workspace_id: "ws_test",
      state: "ready",
      current_phase: "ready",
      phases: [],
      logs: [],
      error: null,
    } as never);
    vi.mocked(importProviderAuthCandidates).mockResolvedValue({ results: [] } as never);
    vi.mocked(desktopConnectLocal).mockResolvedValue({
      kind: "local",
      base_url: "http://127.0.0.1:4402",
      token: "test-token",
    } as never);
    vi.mocked(upsertLauncherRecent).mockResolvedValue([]);
    vi.mocked(desktopListSshHosts).mockResolvedValue([]);
    vi.mocked(desktopTestSsh).mockResolvedValue();
  });

  it("requires location selection before allowing next and advances for local selection", async () => {
    renderPage();
    await screen.findByTestId("workspace-setup");
    expect(wizardStepKey()).toBe("location");

    const nextButton = screen.getByTestId("wizard-next");
    expect(nextButton).toBeDisabled();

    await selectLocalAndContinue();
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });
  });

  it("advances to container without waiting for local preflight checks", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    let resolveScan: ((value: { candidates: never[] }) => void) | null = null;
    const pendingScan = new Promise<{ candidates: never[] }>((resolve) => {
      resolveScan = resolve;
    });
    vi.mocked(listProviderAuthImportCandidates).mockImplementation(() => pendingScan as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    fireEvent.click(screen.getByTestId("wizard-option-location-local"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });

    await act(async () => {
      resolveScan?.({ candidates: [] });
      await pendingScan;
    });

    expect(wizardStepKey()).toBe("container");
  });

  it("shows harness downloads step after container when downloadable providers are missing", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue(configuredTitlingSettingsFixture() as never);
    vi.mocked(listProviders).mockResolvedValue([
      providerStatusFixture({
        provider_id: "codex",
        installed: false,
        health: "error",
        details: { install_supported: "true" },
      }),
    ] as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });

    fireEvent.click(screen.getByTestId("wizard-option-container-disk-isolated"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("harness-downloads");
    });
    expect(screen.getByTestId("wizard-harness-downloads-scroll-shell")).toBeInTheDocument();
    expect(screen.getByTestId("wizard-harness-checkbox-codex")).toBeInTheDocument();
  });

  it("skips harness downloads step when providers are already installed", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue(configuredTitlingSettingsFixture() as never);
    vi.mocked(listProviders).mockResolvedValue([
      providerStatusFixture({
        provider_id: "codex",
        installed: true,
        health: "ok",
        details: { install_supported: "true" },
      }),
    ] as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });

    fireEvent.click(screen.getByTestId("wizard-option-container-disk-isolated"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });
  });

  it("starts selected harness downloads and advances immediately while they continue in background", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue(configuredTitlingSettingsFixture() as never);
    vi.mocked(listProviders).mockResolvedValue([
      providerStatusFixture({
        provider_id: "codex",
        installed: false,
        health: "error",
        details: { install_supported: "true" },
      }),
    ] as never);
    vi.mocked(installProvider).mockResolvedValue({
      provider_id: "codex",
      install_id: "install_codex",
    } as never);
    vi.mocked(getInstall).mockResolvedValue({
      install_id: "install_codex",
      provider_id: "codex",
      state: "running",
      started_at: "2026-02-28T00:00:00Z",
      finished_at: undefined,
      error: undefined,
      last_event: {
        install_id: "install_codex",
        provider_id: "codex",
        at: "2026-02-28T00:00:00Z",
        stage: "download",
        message: "downloading…",
        level: "info",
        bytes: 5,
        total_bytes: 10,
      },
      target: "container",
      error_code: undefined,
    } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });
    fireEvent.click(screen.getByTestId("wizard-option-container-disk-isolated"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("harness-downloads");
    });

    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(installProvider).toHaveBeenCalledWith("codex", "container");
      expect(getInstall).toHaveBeenCalledWith("install_codex");
      expect(wizardStepKey()).toBe("source");
    });
  }, 15000);

  it("continues past harness step when a selected download fails to start", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue(configuredTitlingSettingsFixture() as never);
    vi.mocked(listProviders).mockResolvedValue([
      providerStatusFixture({
        provider_id: "codex",
        installed: false,
        health: "error",
        details: { install_supported: "true" },
      }),
    ] as never);
    vi.mocked(installProvider).mockRejectedValueOnce(new Error("network timed out"));

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });
    fireEvent.click(screen.getByTestId("wizard-option-container-disk-isolated"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("harness-downloads");
    });

    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(installProvider).toHaveBeenCalledWith("codex", "container");
      expect(wizardStepKey()).toBe("source");
    });

    fireEvent.click(screen.getByTestId("wizard-back"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("harness-downloads");
      expect(screen.getByText(/Selected downloads failed to start\./i)).toBeInTheDocument();
      expect(screen.getByText(/Codex: network timed out/i)).toBeInTheDocument();
      expect(screen.getByText(/Continuing without those downloads\./i)).toBeInTheDocument();
    });
  });

  it("never regresses to location while late harness planning resolves", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue(configuredTitlingSettingsFixture() as never);
    let resolveContainerProviders: ((value: unknown) => void) | null = null;
    const pendingContainerProviders = new Promise((resolve) => {
      resolveContainerProviders = resolve;
    });
    vi.mocked(listProviders)
      .mockResolvedValueOnce([
        providerStatusFixture({
          provider_id: "codex",
          installed: true,
          health: "ok",
          details: { install_supported: "true" },
        }),
      ] as never)
      .mockImplementationOnce(() => pendingContainerProviders as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    const trace = captureStepSequence();

    await selectLocalAndContinue();
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });

    fireEvent.click(screen.getByTestId("wizard-option-container-disk-isolated"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
      expect(screen.getByTestId("wizard-next")).toBeDisabled();
    });

    await act(async () => {
      resolveContainerProviders?.([
        providerStatusFixture({
          provider_id: "codex",
          installed: false,
          health: "error",
          details: { install_supported: "true" },
        }),
      ]);
    });

    await waitFor(() => {
      expect(["harness-downloads", "source"]).toContain(wizardStepKey());
    });
    trace.disconnect();
    expect(trace.seen[0]).toBe("location");
    expect(trace.seen.slice(1)).not.toContain("location");
  });

  it("keeps cancel enabled for active harness installs after scan completes", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue(configuredTitlingSettingsFixture() as never);
    vi.mocked(getInstall).mockReset();
    vi.mocked(listProviders).mockResolvedValue([
      providerStatusFixture({
        provider_id: "codex",
        installed: false,
        health: "error",
        details: {
          install_supported: "true",
          install_running: "true",
          install_id: "install_codex",
        },
      }),
    ] as never);
    vi.mocked(getInstall).mockResolvedValue({
      install_id: "install_codex",
      provider_id: "codex",
      state: "running",
      started_at: "2026-02-28T00:00:00Z",
      finished_at: undefined,
      error: undefined,
      last_event: undefined,
      target: "container",
      error_code: undefined,
    } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });
    fireEvent.click(screen.getByTestId("wizard-option-container-disk-isolated"));
    await waitFor(() => {
      expect(["harness-downloads", "source"]).toContain(wizardStepKey());
    });
    if (wizardStepKey() === "source") {
      fireEvent.click(screen.getByTestId("wizard-back"));
      await waitFor(() => {
        expect(wizardStepKey()).toBe("harness-downloads");
      });
    }
    await waitFor(() => {
      expect(getInstall).toHaveBeenCalled();
    });

    const cancelButton = await screen.findByRole("button", { name: "Cancel install" });
    expect(cancelButton).toBeEnabled();
  });

  it("skips the harness step once a tracked install is already terminal failed", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue(configuredTitlingSettingsFixture() as never);
    vi.mocked(getInstall).mockReset();
    vi.mocked(installProvider).mockReset();
    vi.mocked(listProviders)
      .mockResolvedValueOnce([
        providerStatusFixture({
          provider_id: "codex",
          installed: false,
          health: "error",
          details: {
            install_supported: "true",
            install_running: "true",
            install_id: "install_codex",
          },
        }),
      ] as never)
      .mockResolvedValueOnce([
        providerStatusFixture({
          provider_id: "codex",
          installed: false,
          health: "error",
          details: {
            install_supported: "true",
          },
        }),
      ] as never);
    vi.mocked(getInstall).mockResolvedValue({
      install_id: "install_codex",
      provider_id: "codex",
      state: "failed",
      started_at: "2026-02-28T00:00:00Z",
      finished_at: "2026-02-28T00:00:03Z",
      error: "download failed",
      last_event: undefined,
      error_code: "download_failed",
    } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });
    fireEvent.click(screen.getByTestId("wizard-option-container-disk-isolated"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });
    expect(installProvider).not.toHaveBeenCalled();
  });

  it("keeps continue enabled after a selected harness download is canceled", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue(configuredTitlingSettingsFixture() as never);
    vi.mocked(getInstall).mockReset();
    vi.mocked(installProvider).mockReset();
    vi.mocked(listProviders).mockResolvedValue([
      providerStatusFixture({
        provider_id: "codex",
        installed: false,
        health: "error",
        details: {
          install_supported: "true",
          install_running: "true",
          install_id: "install_codex",
        },
      }),
    ] as never);
    vi.mocked(getInstall).mockResolvedValue({
      install_id: "install_codex",
      provider_id: "codex",
      state: "running",
      started_at: "2026-02-28T00:00:00Z",
      finished_at: undefined,
      error: undefined,
      last_event: undefined,
      target: "container",
      error_code: undefined,
    } as never);
    const cancelledInstall = {
      install_id: "install_codex",
      provider_id: "codex",
      state: "cancelled",
      started_at: "2026-02-28T00:00:00Z",
      finished_at: "2026-02-28T00:00:05Z",
      error: "canceled by user",
      last_event: undefined,
      target: "container",
      error_code: undefined,
    };
    vi.mocked(cancelInstall).mockImplementation(async () => {
      vi.mocked(getInstall).mockResolvedValue(cancelledInstall as never);
      return cancelledInstall as never;
    });

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });
    fireEvent.click(screen.getByTestId("wizard-option-container-disk-isolated"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("harness-downloads");
    });
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Cancel install" })).toBeEnabled();
    });

    fireEvent.click(screen.getByRole("button", { name: "Cancel install" }));

    await waitFor(() => {
      expect(cancelInstall).toHaveBeenCalledWith("install_codex");
      expect(screen.getByText(/failed or were canceled/i)).toBeInTheDocument();
      expect(screen.getByTestId("wizard-next")).toBeEnabled();
    });

    fireEvent.click(screen.getByTestId("wizard-next"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });
    expect(installProvider).not.toHaveBeenCalled();
  });

  it("tracks abandonment only on true unmount and uses the latest viewed step", async () => {
    const view = renderPage();
    await screen.findByTestId("workspace-setup");

    expect(trackWizardStartedMock).toHaveBeenCalledWith({ wizardKey: "workspace_setup" });
    expect(trackWizardStepViewedMock).toHaveBeenCalledWith({
      wizardKey: "workspace_setup",
      stepKey: "location",
      stepIndex: 0,
    });

    await selectLocalAndContinue();
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
      expect(trackWizardStepViewedMock).toHaveBeenCalledWith({
        wizardKey: "workspace_setup",
        stepKey: "container",
        stepIndex: 1,
      });
    });
    expect(trackWizardAbandonedMock).not.toHaveBeenCalled();

    view.unmount();

    expect(trackWizardAbandonedMock).toHaveBeenCalledWith(
      expect.objectContaining({
        wizardKey: "workspace_setup",
        lastStepKey: "container",
        lastStepIndex: 1,
      }),
    );
  });

  it("shows desktop-required error for remote verification when not in desktop app", async () => {
    renderPage();
    await screen.findByTestId("workspace-setup");
    fireEvent.click(screen.getByTestId("wizard-option-location-remote"));

    const hostInput = await screen.findByTestId("wizard-remote-host");
    fireEvent.change(hostInput, { target: { value: "devbox.example" } });

    const nextButton = screen.getByTestId("wizard-next");
    expect(nextButton).toBeEnabled();
    fireEvent.click(nextButton);

    expect(
      await screen.findByText("Remote connections require the desktop app."),
    ).toBeInTheDocument();
    expect(wizardStepKey()).toBe("location");
    expect(desktopTestSsh).not.toHaveBeenCalled();
  });

  it("verifies remote SSH in desktop mode without prompting for ctx binary path", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    renderPage();
    await screen.findByTestId("workspace-setup");

    fireEvent.click(screen.getByTestId("wizard-option-location-remote"));
    const hostInput = await screen.findByTestId("wizard-remote-host");
    fireEvent.change(hostInput, { target: { value: "devbox.example" } });

    const nextButton = screen.getByTestId("wizard-next");
    fireEvent.click(nextButton);
    await waitFor(() => {
      expect(desktopTestSsh).toHaveBeenCalledWith({
        host: "devbox.example",
        user: null,
        password_once: null,
      });
    });
    expect(screen.queryByTestId("wizard-remote-password-once")).not.toBeInTheDocument();
  });

  it("keeps remote verification and source-step flow cold before create", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue(configuredTitlingSettingsFixture() as never);
    renderPage();
    await screen.findByTestId("workspace-setup");

    fireEvent.click(screen.getByTestId("wizard-option-location-remote"));
    fireEvent.change(await screen.findByTestId("wizard-remote-host"), {
      target: { value: "devbox.example" },
    });

    fireEvent.click(screen.getByTestId("wizard-next"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
      expect(desktopTestSsh).toHaveBeenCalledWith({
        host: "devbox.example",
        user: null,
        password_once: null,
      });
    });

    fireEvent.click(screen.getByTestId("wizard-option-container-no-container"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });

    fireEvent.click(screen.getByTestId("wizard-option-source-new"));
    fireEvent.change(screen.getByTestId("wizard-source-path"), {
      target: { value: "/remote/new-workspace" },
    });
    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(["session-titling", "setup"]).toContain(wizardStepKey());
    });
    if (wizardStepKey() === "session-titling") {
      fireEvent.click(screen.getByTestId("wizard-titling-skip"));
      await waitFor(() => {
        expect(wizardStepKey()).toBe("setup");
      });
    }

    expect(vi.mocked(desktopConnectSsh).mock.calls.length).toBeGreaterThan(0);
    for (const [req] of vi.mocked(desktopConnectSsh).mock.calls) {
      expect(req.start_remote).toBe(false);
    }
    expect(desktopKickoffRemotePrewarm).not.toHaveBeenCalled();
    expect(startWorkspaceSetupLaunchHandoff).not.toHaveBeenCalled();
    expect(repoValidateDestination).not.toHaveBeenCalled();
  });

  it("ignores hidden saved remote overrides during remote create flow", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue(configuredTitlingSettingsFixture() as never);
    window.localStorage.setItem("contextDesktopRemoteProfilesV1", JSON.stringify([
      {
        host: "devbox.example",
        user: null,
        remote_port: 4411,
        remote_data_dir: "/tmp/ctx-hidden",
        updated_at_ms: 1,
      },
    ]));

    renderPage();
    await screen.findByTestId("workspace-setup");

    fireEvent.click(screen.getByTestId("wizard-option-location-remote"));
    fireEvent.change(await screen.findByTestId("wizard-remote-host"), {
      target: { value: "devbox.example" },
    });

    fireEvent.click(screen.getByTestId("wizard-next"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
      expect(desktopTestSsh).toHaveBeenCalledWith({
        host: "devbox.example",
        user: null,
        password_once: null,
      });
    });

    fireEvent.click(screen.getByTestId("wizard-option-container-no-container"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });

    fireEvent.click(screen.getByTestId("wizard-option-source-new"));
    fireEvent.change(screen.getByTestId("wizard-source-path"), {
      target: { value: "/remote/new-hidden-defaults" },
    });
    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(["session-titling", "setup"]).toContain(wizardStepKey());
      expect(vi.mocked(desktopConnectSsh).mock.calls.length).toBeGreaterThan(0);
    });
    for (const [request] of vi.mocked(desktopConnectSsh).mock.calls) {
      expect(request).toEqual(expect.objectContaining({
        host: "devbox.example",
        user: null,
        password_once: null,
        remote_port: 4399,
        remote_data_dir: null,
      }));
    }
    if (wizardStepKey() === "session-titling") {
      fireEvent.click(screen.getByTestId("wizard-titling-skip"));
      await waitFor(() => {
        expect(wizardStepKey()).toBe("setup");
      });
    }

    fireEvent.click(screen.getByTestId("wizard-next"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("merge-queue");
    });
    fireEvent.click(screen.getByTestId("wizard-next"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("confirm");
    });
    fireEvent.click(screen.getByTestId("wizard-create"));

    await waitFor(() => {
      expect(createWorkspace).toHaveBeenCalledWith(
        "/remote/new-hidden-defaults",
        "new-hidden-defaults",
        "remote",
        "wizard",
      );
      expect(upsertLauncherRecent).toHaveBeenCalledWith(expect.objectContaining({
        kind: "ssh",
        host: "devbox.example",
        user: null,
        remote_port: 4399,
        remote_data_dir: null,
      }));
    });

    expect(JSON.parse(window.localStorage.getItem("contextDesktopRemoteProfilesV1") ?? "[]")).toEqual([
      expect.objectContaining({
        host: "devbox.example",
        user: null,
        remote_port: 4399,
        remote_data_dir: null,
      }),
    ]);
  });

  it("uses the latest visible remote host draft for downstream connects and create", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue(configuredTitlingSettingsFixture() as never);

    renderPage();
    await screen.findByTestId("workspace-setup");

    fireEvent.click(screen.getByTestId("wizard-option-location-remote"));
    const hostInput = await screen.findByTestId("wizard-remote-host");
    fireEvent.change(hostInput, {
      target: { value: "first.example" },
    });

    fireEvent.click(screen.getByTestId("wizard-next"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
      expect(desktopTestSsh).toHaveBeenLastCalledWith({
        host: "first.example",
        user: null,
        password_once: null,
      });
    });
    const firstTargetConnectCallCount = vi.mocked(desktopConnectSsh).mock.calls.length;

    fireEvent.click(screen.getByTestId("wizard-back"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("location");
    });

    fireEvent.change(await screen.findByTestId("wizard-remote-host"), {
      target: { value: "second.example" },
    });
    fireEvent.click(screen.getByTestId("wizard-next"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
      expect(desktopTestSsh).toHaveBeenLastCalledWith({
        host: "second.example",
        user: null,
        password_once: null,
      });
    });

    fireEvent.click(screen.getByTestId("wizard-option-container-no-container"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });

    fireEvent.click(screen.getByTestId("wizard-option-source-new"));
    fireEvent.change(screen.getByTestId("wizard-source-path"), {
      target: { value: "/remote/latest-visible-target" },
    });
    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("setup");
    });

    fireEvent.click(screen.getByTestId("wizard-next"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("merge-queue");
    });
    fireEvent.click(screen.getByTestId("wizard-next"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("confirm");
    });
    fireEvent.click(screen.getByTestId("wizard-create"));

    await waitFor(() => {
      expect(createWorkspace).toHaveBeenCalledWith(
        "/remote/latest-visible-target",
        "latest-visible-target",
        "remote",
        "wizard",
      );
      expect(upsertLauncherRecent).toHaveBeenCalledWith(expect.objectContaining({
        kind: "ssh",
        host: "second.example",
        user: null,
        remote_port: 4399,
        remote_data_dir: null,
      }));
    });

    const retargetedCalls = vi.mocked(desktopConnectSsh).mock.calls.slice(firstTargetConnectCallCount);
    expect(retargetedCalls.length).toBeGreaterThan(0);
    for (const [request] of retargetedCalls) {
      expect(request).toEqual(expect.objectContaining({
        host: "second.example",
        user: null,
        remote_port: 4399,
        remote_data_dir: null,
      }));
    }
  });

  it("asks for one-time SSH password only after key-auth failure", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(desktopTestSsh)
      .mockRejectedValueOnce(new Error("ssh failed to probe remote platform: Permission denied (publickey,password)."))
      .mockResolvedValueOnce();
    renderPage();
    await screen.findByTestId("workspace-setup");

    fireEvent.click(screen.getByTestId("wizard-option-location-remote"));
    fireEvent.change(await screen.findByTestId("wizard-remote-host"), {
      target: { value: "devbox.example" },
    });
    expect(screen.queryByTestId("wizard-remote-password-once")).not.toBeInTheDocument();

    const nextButton = screen.getByTestId("wizard-next");
    fireEvent.click(nextButton);
    await screen.findByTestId("wizard-remote-password-once");
    expect(screen.getByText("Used to install SSH key auth; never stored")).toBeInTheDocument();
    expect(desktopTestSsh).toHaveBeenNthCalledWith(1, {
      host: "devbox.example",
      user: null,
      password_once: null,
    });

    fireEvent.change(screen.getByTestId("wizard-remote-password-once"), {
      target: { value: "hunter2" },
    });
    fireEvent.click(nextButton);
    await waitFor(() => {
      expect(desktopTestSsh).toHaveBeenNthCalledWith(2, {
        host: "devbox.example",
        user: null,
        password_once: "hunter2",
      });
    });
  });

  it("shows explicit unsupported message when remote probe rejects Windows hosts", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(desktopTestSsh).mockRejectedValueOnce(
      new Error("Remote Windows hosts are not supported yet. Use a Linux host (x86_64 or arm64)."),
    );
    renderPage();
    await screen.findByTestId("workspace-setup");

    fireEvent.click(screen.getByTestId("wizard-option-location-remote"));
    fireEvent.change(await screen.findByTestId("wizard-remote-host"), {
      target: { value: "win-host.example" },
    });
    fireEvent.click(screen.getByTestId("wizard-next"));

    expect(
      await screen.findByText(
        "Remote Windows hosts are not supported yet. Use a Linux host (x86_64 or arm64).",
      ),
    ).toBeInTheDocument();
  });

  it("does not regress step when stale local prefetch resolves after advancing", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    let resolveScan: ((value: { candidates: never[] }) => void) | null = null;
    const pendingScan = new Promise<{ candidates: never[] }>((resolve) => {
      resolveScan = resolve;
    });
    vi.mocked(listProviderAuthImportCandidates).mockImplementation(() => pendingScan as never);

    renderPage();
    await screen.findByTestId("workspace-setup");

    fireEvent.click(screen.getByTestId("wizard-option-location-local"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
      expect(listProviderAuthImportCandidates).toHaveBeenCalled();
    });

    await act(async () => {
      resolveScan?.({ candidates: [] });
      await pendingScan;
    });

    expect(wizardStepKey()).toBe("container");
  });

  it("defers a failed local auth probe after one unavailable-daemon attempt", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(desktopConnectLocal).mockRejectedValue(new Error("local daemon unavailable") as never);

    renderPage();
    await screen.findByTestId("workspace-setup");

    await waitFor(() => {
      expect(desktopConnectLocal).toHaveBeenCalledTimes(1);
      expect(wizardStepKey()).toBe("location");
    });

    await act(async () => {
      await new Promise((resolve) => window.setTimeout(resolve, 25));
    });

    expect(desktopConnectLocal).toHaveBeenCalledTimes(1);
  });

  it("retries deferred local auth and harness scans after a later successful local daemon connect", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue(configuredTitlingSettingsFixture() as never);
    vi.mocked(listProviderAuthImportCandidates).mockResolvedValue({
      candidates: [
        {
          id: "cand-codex",
          provider_id: "codex",
          provider_label: "Codex",
          kind: "auth_file",
          path: "/Users/test/.codex/auth.json",
          signal_strength: "strong",
          confidence: "high",
          parse_status: "parsed",
        },
      ],
    } as never);
    vi.mocked(listProviders).mockResolvedValue([
      providerStatusFixture({
        provider_id: "codex",
        installed: false,
        health: "error",
        details: {
          install_supported: "true",
        },
      }),
    ] as never);

    const localInfo = {
      kind: "local",
      base_url: "http://127.0.0.1:4402",
      token: "test-token",
    } as const;
    let localDaemonAvailable = false;
    vi.mocked(desktopConnectLocal).mockImplementation(async () => {
      if (!localDaemonAvailable) {
        throw new Error("local daemon unavailable");
      }
      return localInfo as never;
    });

    renderPage();
    await screen.findByTestId("workspace-setup");

    fireEvent.click(screen.getByTestId("wizard-option-location-local"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });

    fireEvent.click(screen.getByTestId("wizard-option-container-no-container"));
    await waitFor(() => {
      expect(["session-titling", "source"]).toContain(wizardStepKey());
    });
    if (wizardStepKey() === "session-titling") {
      fireEvent.click(screen.getByTestId("wizard-titling-skip"));
      await waitFor(() => {
        expect(wizardStepKey()).toBe("source");
      });
    }

    localDaemonAvailable = true;
    fireEvent.click(screen.getByTestId("wizard-option-source-new"));
    fireEvent.change(screen.getByTestId("wizard-source-path"), {
      target: { value: "/tmp/local-retry-workspace" },
    });
    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("harness-downloads");
    });

    fireEvent.click(screen.getByTestId("wizard-harness-skip"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("auth-import");
    });
  });

  it("shows destination validation errors on source next before create", async () => {
    vi.mocked(repoValidateDestination).mockRejectedValueOnce(
      new Error("destination is not empty: /tmp/existing"),
    );

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });

    fireEvent.click(screen.getByTestId("wizard-option-container-no-container"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });

    fireEvent.click(screen.getByTestId("wizard-option-source-new"));
    fireEvent.change(screen.getByTestId("wizard-source-path"), {
      target: { value: "/tmp/existing" },
    });
    fireEvent.click(screen.getByTestId("wizard-next"));

    expect(
      await screen.findByText("destination is not empty: /tmp/existing"),
    ).toBeInTheDocument();
    expect(wizardStepKey()).toBe("source");
    expect(repoValidateDestination).toHaveBeenCalledWith({
      path: "/tmp/existing",
      require_empty_if_exists: true,
    });
  });

  it("preflights clone destination path on source next", async () => {
    vi.mocked(repoValidateDestination).mockRejectedValueOnce(
      new Error("destination already exists: /tmp/projects/repo"),
    );

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });

    fireEvent.click(screen.getByTestId("wizard-option-container-no-container"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });

    fireEvent.click(screen.getByTestId("wizard-option-source-clone"));
    fireEvent.change(screen.getByTestId("wizard-repo-url"), {
      target: { value: "https://github.com/acme/repo.git" },
    });
    fireEvent.change(screen.getByTestId("wizard-source-path"), {
      target: { value: "/tmp/projects/" },
    });
    fireEvent.click(screen.getByTestId("wizard-next"));

    expect(
      await screen.findByText("destination already exists: /tmp/projects/repo"),
    ).toBeInTheDocument();
    expect(wizardStepKey()).toBe("source");
    expect(repoValidateDestination).toHaveBeenCalledWith({
      path: "/tmp/projects/repo",
      must_not_exist: true,
    });
  });

  it("advances from source when preflight passes", async () => {
    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });

    fireEvent.click(screen.getByTestId("wizard-option-container-no-container"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });

    fireEvent.click(screen.getByTestId("wizard-option-source-new"));
    fireEvent.change(screen.getByTestId("wizard-source-path"), {
      target: { value: "/tmp/new-workspace" },
    });
    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("setup");
    });
    expect(repoValidateDestination).toHaveBeenCalledWith({
      path: "/tmp/new-workspace",
      require_empty_if_exists: true,
    });
  });

  it("keeps import source flow reachable for non-repo folders", async () => {
    vi.mocked(repoStatus).mockResolvedValueOnce({
      canonical_path: "/tmp/existing-folder",
      is_repo: false,
      error: "not a git repository",
    } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });

    fireEvent.click(screen.getByTestId("wizard-option-container-no-container"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });

    fireEvent.click(screen.getByTestId("wizard-option-source-import"));
    fireEvent.change(screen.getByTestId("wizard-source-path"), {
      target: { value: "/tmp/existing-folder" },
    });
    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("setup");
    });
    expect(repoStatus).toHaveBeenCalledWith({ path: "/tmp/existing-folder" });
  });

  it("renders only importable auth rows with parsed harnesses preselected", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(listProviderAuthImportCandidates).mockResolvedValue({
      candidates: [
        {
          id: "cand-codex",
          provider_id: "codex",
          provider_label: "Codex",
          kind: "auth_file",
          path: "/Users/example-user/.codex/auth.json",
          signal_strength: "strong",
          confidence: "high",
          parse_status: "parsed",
        },
        {
          id: "cand-cursor",
          provider_id: "cursor",
          provider_label: "Cursor",
          kind: "config_file",
          path: "/Users/example-user/.cursor/cli-config.json",
          signal_strength: "weak",
          confidence: "low-medium",
          parse_status: "unsupported",
          unsupported_reason: "Unsupported in this flow.",
        },
      ],
    } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await advancePastContainerForHost();

    await waitFor(() => {
      expect(wizardStepKey()).toBe("auth-import");
    });

    const codexCheckbox = screen.getByRole("checkbox", { name: /codex/i }) as HTMLInputElement;
    expect(codexCheckbox.checked).toBe(true);
    expect(screen.queryByRole("checkbox", { name: /claude code/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("checkbox", { name: /cursor/i })).not.toBeInTheDocument();
    expect(screen.queryByText("Source: /Users/example-user/.cursor/cli-config.json")).not.toBeInTheDocument();
    expect(screen.getByText("Source: /Users/example-user/.codex/auth.json")).toBeInTheDocument();
  });

  it("keeps auth-import next enabled before session titling is selected", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);
    vi.mocked(listProviderAuthImportCandidates).mockResolvedValue({
      candidates: [
        {
          id: "cand-codex",
          provider_id: "codex",
          provider_label: "Codex",
          kind: "auth_file",
          path: "/Users/example-user/.codex/auth.json",
          signal_strength: "strong",
          confidence: "high",
          parse_status: "parsed",
        },
      ],
    } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await advancePastContainerForHost();

    await waitFor(() => {
      expect(wizardStepKey()).toBe("auth-import");
    });

    const nextButton = screen.getByTestId("wizard-next");
    expect(nextButton).toBeEnabled();
    fireEvent.click(nextButton);

    await waitFor(() => {
      expect(importProviderAuthCandidates).toHaveBeenCalled();
      expect(wizardStepKey()).toBe("session-titling");
    });
  });

  it("surfaces auth import failures and stays on auth-import step", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);
    vi.mocked(listProviderAuthImportCandidates).mockResolvedValue({
      candidates: [
        {
          id: "cand-claude",
          provider_id: "claude-crp",
          provider_label: "Claude Code",
          kind: "auth_file",
          path: "/Users/example-user/.claude.json",
          signal_strength: "strong",
          confidence: "high",
          parse_status: "parsed",
        },
      ],
    } as never);
    vi.mocked(importProviderAuthCandidates).mockResolvedValue({
      results: [
        {
          candidate_id: "cand-claude",
          provider_id: "claude-crp",
          status: "unsupported",
          message: "Could not find ANTHROPIC auth token in candidate file.",
        },
      ],
    } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await advancePastContainerForHost();

    await waitFor(() => {
      expect(wizardStepKey()).toBe("auth-import");
    });

    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(importProviderAuthCandidates).toHaveBeenCalledWith(["cand-claude"]);
      expect(
        screen.getByText(/Some auth imports did not apply\./i),
      ).toBeInTheDocument();
      expect(wizardStepKey()).toBe("auth-import");
    });
  });

  it("probes titling in background while auth-import is available", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);
    vi.mocked(listProviderAuthImportCandidates).mockResolvedValue({
      candidates: [
        {
          id: "cand-codex",
          provider_id: "codex",
          provider_label: "Codex",
          kind: "auth_file",
          path: "/Users/example-user/.codex/auth.json",
          signal_strength: "strong",
          confidence: "high",
          parse_status: "parsed",
        },
      ],
    } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await advancePastContainerForHost();

    await waitFor(() => {
      expect(wizardStepKey()).toBe("auth-import");
    });
    expect(getSettings).toHaveBeenCalled();

    fireEvent.click(screen.getByTestId("wizard-next"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("session-titling");
    });
    expect(getSettings).toHaveBeenCalled();
  });

  it("probes titling when skipping auth-import", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);
    vi.mocked(listProviderAuthImportCandidates).mockResolvedValue({
      candidates: [
        {
          id: "cand-codex",
          provider_id: "codex",
          provider_label: "Codex",
          kind: "auth_file",
          path: "/Users/example-user/.codex/auth.json",
          signal_strength: "strong",
          confidence: "high",
          parse_status: "parsed",
        },
      ],
    } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await advancePastContainerForHost();

    await waitFor(() => {
      expect(wizardStepKey()).toBe("auth-import");
    });
    expect(getSettings).toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Skip for now" }));

    await waitFor(() => {
      expect(importProviderAuthCandidates).not.toHaveBeenCalled();
      expect(wizardStepKey()).toBe("session-titling");
    });
    expect(getSettings).toHaveBeenCalled();
  });

  it("does not launch a second probe while same-target probe is already running", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(listProviderAuthImportCandidates).mockResolvedValue({
      candidates: [
        {
          id: "cand-codex",
          provider_id: "codex",
          provider_label: "Codex",
          kind: "auth_file",
          path: "/Users/example-user/.codex/auth.json",
          signal_strength: "strong",
          confidence: "high",
          parse_status: "parsed",
        },
      ],
    } as never);

    const pendingSettings = new Promise((resolve) => {
      window.setTimeout(() => {
        resolve({ title_generation: null });
      }, 40);
    });
    vi.mocked(getSettings).mockImplementation(() => pendingSettings as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await advancePastContainerForHost();

    await waitFor(() => {
      expect(wizardStepKey()).toBe("auth-import");
    });

    fireEvent.click(screen.getByTestId("wizard-next"));
    await waitFor(() => {
      expect(importProviderAuthCandidates).toHaveBeenCalled();
    });
    expect(getSettings).toHaveBeenCalled();

    await waitFor(() => {
      expect(wizardStepKey()).toBe("session-titling");
    });
    expect(getSettings).toHaveBeenCalled();
  });

  it("does not double-advance when local auto-advance and manual next race", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    let resolveScan: ((value: {
      candidates: Array<{
        id: string;
        provider_id: string;
        provider_label: string;
        kind: string;
        path: string;
        signal_strength: string;
        confidence: string;
        parse_status: string;
      }>;
    }) => void) | null = null;
    const pendingScan = new Promise<{
      candidates: Array<{
        id: string;
        provider_id: string;
        provider_label: string;
        kind: string;
        path: string;
        signal_strength: string;
        confidence: string;
        parse_status: string;
      }>;
    }>((resolve) => {
      resolveScan = resolve;
    });
    vi.mocked(listProviderAuthImportCandidates).mockImplementation(() => pendingScan as never);

    renderPage();
    await screen.findByTestId("workspace-setup");

    fireEvent.click(screen.getByTestId("wizard-option-location-local"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
      expect(listProviderAuthImportCandidates).toHaveBeenCalled();
    });

    fireEvent.click(screen.getByTestId("wizard-option-container-no-container"));

    await act(async () => {
      resolveScan?.({
        candidates: [
          {
            id: "cand-codex",
            provider_id: "codex",
            provider_label: "Codex",
            kind: "auth_file",
            path: "/Users/example-user/.codex/auth.json",
            signal_strength: "strong",
            confidence: "high",
            parse_status: "parsed",
          },
        ],
      });
      await pendingScan;
    });

    await waitFor(() => {
      expect(wizardStepKey()).toBe("auth-import");
    });
  });

  it("continues to titling when no auth candidates are selected", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(listProviderAuthImportCandidates).mockResolvedValue({
      candidates: [
        {
          id: "cand-codex",
          provider_id: "codex",
          provider_label: "Codex",
          kind: "auth_file",
          path: "/Users/example-user/.codex/auth.json",
          signal_strength: "strong",
          confidence: "high",
          parse_status: "parsed",
        },
      ],
    } as never);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await advancePastContainerForHost();

    await waitFor(() => {
      expect(wizardStepKey()).toBe("auth-import");
    });

    fireEvent.click(screen.getByRole("checkbox", { name: /codex/i }));
    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(importProviderAuthCandidates).not.toHaveBeenCalled();
      expect(getSettings).toHaveBeenCalled();
      expect(wizardStepKey()).toBe("session-titling");
    });
  });

  it("shows session titling step when selected daemon is not configured", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await advancePastContainerForHost();

    if (wizardStepKey() === "auth-import") {
      fireEvent.click(screen.getByRole("button", { name: "Skip for now" }));
    }

    await waitFor(() => {
      expect(wizardStepKey()).toBe("session-titling");
    });
    expect(screen.getByTestId("wizard-titling-mode-remote")).toBeInTheDocument();
    expect(screen.getByTestId("wizard-titling-mode-local")).toBeInTheDocument();
  });

  it("holds on container until async titling planning resolves, then advances once", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(listProviderAuthImportCandidates).mockResolvedValue({ candidates: [] } as never);
    let resolveSettings: ((value: unknown) => void) | null = null;
    const pendingSettings = new Promise((resolve) => {
      resolveSettings = resolve;
    });
    vi.mocked(getSettings).mockImplementation(() => pendingSettings as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });
    fireEvent.click(screen.getByTestId("wizard-option-container-no-container"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
      expect(screen.getByTestId("wizard-next")).toBeDisabled();
    });

    await act(async () => {
      resolveSettings?.({ title_generation: null });
    });

    await waitFor(() => {
      expect(wizardStepKey()).toBe("session-titling");
    });
  });

  it("does not show local titling download banner when local mode is disabled", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({
      title_generation: {
        mode: "local",
        remote: {
          base_url: "https://openrouter.ai/api/v1",
          api_key: "",
          model: "google/gemini-3-flash-preview",
          use_json: true,
        },
        local: {
          model_id: "ggml-org/Qwen3-1.7B-GGUF",
          use_json: true,
        },
      },
    } as never);
    vi.mocked(getTitleGenerationLocalStatus).mockResolvedValue({
      ready: false,
      runtime: { version: "0.0.0-test", installed: true, path: "/tmp/runtime" },
      model: {
        model_id: "ggml-org/Qwen3-1.7B-GGUF",
        file_name: "Qwen3-1.7B-GGUF.gguf",
        installed: false,
        version: null,
        sha256: null,
        size_bytes: null,
        installed_at: null,
      },
      install_id: "install_test",
      install_running: true,
    } as never);
    vi.mocked(getInstall).mockResolvedValue({
      install_id: "install_test",
      provider_id: "title_generation_local",
      state: "running",
      started_at: "2026-02-18T00:00:00Z",
      last_event: {
        install_id: "install_test",
        provider_id: "title_generation_local",
        at: "2026-02-18T00:00:01Z",
        stage: "download_model",
        message: "Downloading model file",
        level: "info",
        bytes: 1,
        total_bytes: 2,
      },
    } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();

    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });
    expect(screen.queryByText("Session titling model download in progress.")).not.toBeInTheDocument();
  });

  it("masks session titling remote API key input", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await advanceToTitlingStep();
    fireEvent.click(screen.getByTestId("wizard-titling-mode-remote"));

    const apiKeyInput = screen.getByTestId("wizard-titling-remote-api-key");
    expect(apiKeyInput).toHaveAttribute("type", "password");
  });

  it("skips session titling step when daemon is already configured and ready", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({
      title_generation: {
        mode: "remote",
        remote: {
          base_url: "https://openrouter.ai/api/v1",
          api_key: "sk-ready",
          model: "google/gemini-3-flash-preview",
          use_json: true,
        },
        local: {
          model_id: "ggml-org/Qwen3-1.7B-GGUF",
          use_json: true,
        },
      },
    } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();

    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });
    expect(screen.queryByTestId("wizard-titling-mode-remote")).not.toBeInTheDocument();
    expect(updateSettings).not.toHaveBeenCalled();
  });

  it("persists remote session titling settings before advancing", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await advanceToTitlingStep();
    fireEvent.click(screen.getByTestId("wizard-titling-mode-remote"));
    fireEvent.change(screen.getByTestId("wizard-titling-remote-base-url"), {
      target: { value: "https://api.example/v1" },
    });
    fireEvent.change(screen.getByTestId("wizard-titling-remote-model"), {
      target: { value: "provider/model-name" },
    });
    fireEvent.change(screen.getByTestId("wizard-titling-remote-api-key"), {
      target: { value: "sk-onboarding" },
    });
    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(updateSettings).toHaveBeenCalledWith(expect.objectContaining({
        title_generation: expect.objectContaining({
          mode: "remote",
          remote: expect.objectContaining({
            api_key: "sk-onboarding",
          }),
        }),
      }));
    });
    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });
  });

  it("keeps local titling option visible but disabled as coming soon", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await advanceToTitlingStep();
    const localButton = screen.getByTestId("wizard-titling-mode-local");
    expect(localButton).toBeDisabled();
    expect(screen.getByText("Coming soon: download a small LLM to run locally for generating task titles.")).toBeInTheDocument();
    fireEvent.click(localButton);
    expect(installTitleGenerationLocal).not.toHaveBeenCalled();
    expect(wizardStepKey()).toBe("session-titling");
  });

  it("removes configurable local titling inputs from the wizard step", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await advanceToTitlingStep();
    expect(screen.queryByTestId("wizard-titling-local-model-id")).not.toBeInTheDocument();
    expect(screen.queryByTestId("wizard-titling-local-install")).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId("wizard-titling-mode-remote"));
    expect(screen.queryByText("Base URL, API key, and model are required.")).not.toBeInTheDocument();
  });

  it("supports skip path without writing session titling settings", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await advanceToTitlingStep();
    fireEvent.click(screen.getByTestId("wizard-titling-skip"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });
    expect(updateSettings).not.toHaveBeenCalled();
  });

  it("keeps auth import in the cached route plan after an explicit titling skip", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);
    vi.mocked(listProviderAuthImportCandidates).mockResolvedValue({
      candidates: [
        {
          id: "cand-codex",
          provider_id: "codex",
          provider_label: "Codex",
          kind: "auth_file",
          path: "/Users/example-user/.codex/auth.json",
          signal_strength: "strong",
          confidence: "high",
          parse_status: "parsed",
        },
      ],
    } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await advanceToTitlingStep();
    fireEvent.click(screen.getByTestId("wizard-titling-skip"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });

    fireEvent.click(screen.getByTestId("wizard-back"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("auth-import");
    });
  });

  it("does not reinsert session titling after an explicit skip when auth import advances again", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);
    vi.mocked(listProviderAuthImportCandidates).mockResolvedValue({
      candidates: [
        {
          id: "cand-codex",
          provider_id: "codex",
          provider_label: "Codex",
          kind: "auth_file",
          path: "/Users/example-user/.codex/auth.json",
          signal_strength: "strong",
          confidence: "high",
          parse_status: "parsed",
        },
      ],
    } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await advanceToTitlingStep();
    fireEvent.click(screen.getByTestId("wizard-titling-skip"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });

    fireEvent.click(screen.getByTestId("wizard-back"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("auth-import");
    });

    fireEvent.click(screen.getByTestId("wizard-next"));
    await waitFor(() => {
      expect(importProviderAuthCandidates).not.toHaveBeenCalled();
      expect(wizardStepKey()).toBe("source");
    });
  });

  it("preserves an explicit titling skip across later force re-probes and route-plan recalculations", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);
    vi.mocked(listProviderAuthImportCandidates).mockResolvedValue({ candidates: [] } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await advanceToTitlingStep();
    fireEvent.click(screen.getByTestId("wizard-titling-skip"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });

    fireEvent.click(screen.getByTestId("wizard-option-source-new"));
    fireEvent.change(screen.getByTestId("wizard-source-path"), {
      target: { value: "/tmp/durable-skip-repro" },
    });
    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("setup");
    });

    fireEvent.click(screen.getByTestId("wizard-back"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });

    fireEvent.click(screen.getByTestId("wizard-back"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });

    fireEvent.click(screen.getByTestId("wizard-next"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });
  });

  it("creates a local workspace in browser mode without desktop bridge connect", async () => {
    renderPage();
    await screen.findByTestId("workspace-setup");

    fireEvent.click(screen.getByTestId("wizard-option-location-local"));
    await waitFor(() => {
      expect(wizardStepKey()).not.toBe("location");
    });

    if (wizardStepKey() === "auth-import") {
      fireEvent.click(screen.getByRole("button", { name: "Skip for now" }));
      await waitFor(() => {
        expect(wizardStepKey()).toBe("session-titling");
      });
    }
    if (wizardStepKey() === "session-titling") {
      fireEvent.click(screen.getByTestId("wizard-titling-skip"));
    }

    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });
    fireEvent.click(screen.getByTestId("wizard-option-container-no-container"));
    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });
    fireEvent.click(screen.getByTestId("wizard-option-source-new"));
    fireEvent.change(screen.getByTestId("wizard-source-path"), {
      target: { value: "/tmp/new-repo-web" },
    });
    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("setup");
    });
    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("merge-queue");
    });
    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("confirm");
    });
    fireEvent.click(screen.getByTestId("wizard-create"));

    await waitFor(() => {
      expect(createWorkspace).toHaveBeenCalledWith("/tmp/new-repo-web", "new-repo-web", "local", "wizard");
      expect(desktopConnectLocal).not.toHaveBeenCalled();
      expect(upsertLauncherRecent).toHaveBeenCalledWith(expect.objectContaining({
        kind: "local",
        root_path: "/tmp/new-repo-web",
      }));
    });
  });

  it("writes launcher recents on successful local workspace creation", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await advancePastContainerForHost();

    if (wizardStepKey() === "auth-import") {
      fireEvent.click(screen.getByRole("button", { name: "Skip for now" }));
    }
    if (wizardStepKey() === "session-titling") {
      fireEvent.click(screen.getByTestId("wizard-titling-skip"));
    }

    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });
    fireEvent.click(screen.getByTestId("wizard-option-source-new"));
    fireEvent.change(screen.getByTestId("wizard-source-path"), {
      target: { value: "/tmp/new-repo" },
    });
    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("setup");
    });
    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("merge-queue");
    });
    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("confirm");
    });
    fireEvent.click(screen.getByTestId("wizard-create"));

    await waitFor(() => {
      expect(createWorkspace).toHaveBeenCalled();
      expect(upsertLauncherRecent).toHaveBeenCalledWith(expect.objectContaining({
        kind: "local",
        root_path: "/tmp/new-repo",
        label: "new-repo",
        updated_at_ms: expect.any(Number),
      }));
    });
  });

  it("creates container workspace while optional harness downloads are still running", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue(configuredTitlingSettingsFixture() as never);
    vi.mocked(listProviders).mockResolvedValue([
      providerStatusFixture({
        provider_id: "codex",
        installed: false,
        health: "error",
        details: { install_supported: "true" },
      }),
    ] as never);
    vi.mocked(installProvider).mockResolvedValue({
      provider_id: "codex",
      install_id: "install_codex",
    } as never);
    vi.mocked(getInstall).mockResolvedValue({
      install_id: "install_codex",
      provider_id: "codex",
      state: "running",
      started_at: "2026-02-28T00:00:00Z",
      finished_at: undefined,
      error: undefined,
      last_event: {
        install_id: "install_codex",
        provider_id: "codex",
        at: "2026-02-28T00:00:00Z",
        stage: "download",
        message: "downloading…",
        level: "info",
        bytes: 5,
        total_bytes: 10,
      },
      target: "container",
      error_code: undefined,
    } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    await selectLocalAndContinue();
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });
    fireEvent.click(screen.getByTestId("wizard-option-container-disk-isolated"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("harness-downloads");
    });

    fireEvent.click(screen.getByTestId("wizard-next"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });

    fireEvent.click(screen.getByTestId("wizard-option-source-new"));
    fireEvent.change(screen.getByTestId("wizard-workspace-name"), {
      target: { value: "disk-isolated-bg" },
    });
    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("network");
    });
    fireEvent.click(screen.getByTestId("wizard-option-network-full"));

    await waitFor(() => {
      expect(["setup", "merge-queue"]).toContain(wizardStepKey());
    });
    if (wizardStepKey() === "setup") {
      fireEvent.click(screen.getByTestId("wizard-next"));
      await waitFor(() => {
        expect(wizardStepKey()).toBe("merge-queue");
      });
    }

    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("confirm");
      expect(screen.getByText(/1 selected download in progress/i)).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("wizard-create"));

    await waitFor(() => {
      expect(createWorkspace).toHaveBeenCalled();
      expect(startWorkspaceSetupLaunchHandoff).toHaveBeenCalled();
      expect(trackWorkspaceLaunchCompletedMock).toHaveBeenNthCalledWith(
        1,
        expect.objectContaining({
          workspaceId: "ws_test",
          workspaceKind: "local",
          executionMode: "container",
          result: "ready",
          persistPendingRoute: false,
        }),
      );
      expect(trackWorkspaceLaunchCompletedMock).toHaveBeenNthCalledWith(
        2,
        expect.objectContaining({
          workspaceId: "ws_test",
          workspaceKind: "local",
          executionMode: "container",
          result: "ready",
          emitEvent: false,
        }),
      );
    });
  });
});
