import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import WorkspaceSetupPage from "./WorkspaceSetupPage";
import {
  buildExecutionLaunchWsUrl,
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
  startExecutionLaunch,
  startExecutionRuntimePrewarm,
  updateSettings,
  updateWorkspaceExecutionConfig,
  updateWorkspaceMergeQueueConfig,
  updateWorkspaceWorktreeBootstrapConfig,
} from "../api/client";
import { desktopConnectLocal, desktopListSshHosts, desktopTestSsh, isDesktopApp } from "../utils/desktop";
import { upsertLauncherRecent } from "../state/launcherRecentsStore";

const {
  trackWizardStartedMock,
  trackWizardStepViewedMock,
  trackWizardStepCompletedMock,
  trackWizardCompletedMock,
  trackWizardAbandonedMock,
} = vi.hoisted(() => ({
  trackWizardStartedMock: vi.fn(),
  trackWizardStepViewedMock: vi.fn(),
  trackWizardStepCompletedMock: vi.fn(),
  trackWizardCompletedMock: vi.fn(),
  trackWizardAbandonedMock: vi.fn(),
}));

vi.mock("../api/client", async () => {
  const actual = await vi.importActual<typeof import("../api/client")>("../api/client");
  return {
    ...actual,
    applyDaemonDesktopConnection: vi.fn(),
    buildExecutionLaunchWsUrl: vi.fn(),
    createWorkspace: vi.fn(),
    getInstall: vi.fn(),
    getExecutionLaunchStatus: vi.fn(),
    getHealth: vi.fn(),
    getSettings: vi.fn(),
    installProvider: vi.fn(),
    listProviders: vi.fn(),
    getTitleGenerationLocalStatus: vi.fn(),
    importProviderAuthCandidates: vi.fn(),
    installTitleGenerationLocal: vi.fn(),
    listProviderAuthImportCandidates: vi.fn(),
    listWorkspaces: vi.fn(),
    repoClone: vi.fn(),
    repoInit: vi.fn(),
    repoStatus: vi.fn(),
    repoValidateDestination: vi.fn(),
    repoStagingPath: vi.fn(),
    startExecutionLaunch: vi.fn(),
    startExecutionRuntimePrewarm: vi.fn(),
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
    vi.useRealTimers();
  });

  beforeEach(() => {
    vi.clearAllMocks();
    trackWizardStartedMock.mockReset();
    trackWizardStepViewedMock.mockReset();
    trackWizardStepCompletedMock.mockReset();
    trackWizardCompletedMock.mockReset();
    trackWizardAbandonedMock.mockReset();
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
    vi.mocked(startExecutionLaunch).mockResolvedValue({
      job_id: "job_test",
      workspace_id: "ws_test",
      kind: "workspace_launch",
      state: "ready",
      current_phase: "ready",
      phases: [],
      logs: [],
      error: null,
    } as never);
    vi.mocked(startExecutionRuntimePrewarm).mockResolvedValue({
      job_id: "job_prewarm_test",
      workspace_id: "00000000-0000-0000-0000-000000000000",
      kind: "startup_prewarm",
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

  it("starts selected harness downloads and stays on the step until they finish", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue(configuredTitlingSettingsFixture() as never);
    let scanCount = 0;
    vi.mocked(listProviders).mockImplementation(async () => {
      scanCount += 1;
      if (scanCount >= 3) {
        return [
          providerStatusFixture({
            provider_id: "codex",
            installed: true,
            health: "ok",
            details: { install_supported: "true" },
          }),
        ] as never;
      }
      return [
        providerStatusFixture({
          provider_id: "codex",
          installed: false,
          health: "error",
          details: { install_supported: "true" },
        }),
      ] as never;
    });
    vi.mocked(installProvider).mockResolvedValue({
      provider_id: "codex",
      install_id: "install_codex",
    } as never);
    vi.mocked(getInstall)
      .mockResolvedValueOnce({
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
      } as never)
      .mockResolvedValueOnce({
        install_id: "install_codex",
        provider_id: "codex",
        state: "succeeded",
        started_at: "2026-02-28T00:00:00Z",
        finished_at: "2026-02-28T00:00:02Z",
        error: undefined,
        last_event: {
          install_id: "install_codex",
          provider_id: "codex",
          at: "2026-02-28T00:00:02Z",
          stage: "done",
          message: "Install complete",
          level: "success",
        },
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
      expect(wizardStepKey()).toBe("harness-downloads");
      expect(screen.getByText(/Selected downloads are still running/i)).toBeInTheDocument();
      expect(screen.getByTestId("wizard-next")).toBeDisabled();
    });

    await act(async () => {
      await new Promise((resolve) => window.setTimeout(resolve, 1000));
    });

    await waitFor(() => {
      expect(scanCount).toBeGreaterThanOrEqual(3);
      expect(screen.getByText(/Installed · container/i)).toBeInTheDocument();
      expect(screen.getByTestId("wizard-next")).toBeEnabled();
    });

    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });
  }, 15000);

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

  it("clears stale running state after terminal harness install failures", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue(configuredTitlingSettingsFixture() as never);
    vi.mocked(getInstall).mockReset();
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
      expect(wizardStepKey()).toBe("harness-downloads");
    });

    await waitFor(() => {
      expect(screen.getByText(/Not installed · container/i)).toBeInTheDocument();
    });
    expect(screen.getByTestId("wizard-harness-checkbox-codex")).toBeEnabled();
    expect(screen.getByTestId("wizard-next")).toBeDisabled();
    expect(screen.getByText(/failed or were canceled/i)).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("wizard-harness-skip"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("source");
    });
  });

  it("tracks wizard start, step viewed, and abandonment on unmount", async () => {
    const view = renderPage();
    await screen.findByTestId("workspace-setup");

    expect(trackWizardStartedMock).toHaveBeenCalledWith({ wizardKey: "workspace_setup" });
    expect(trackWizardStepViewedMock).toHaveBeenCalledWith({
      wizardKey: "workspace_setup",
      stepKey: "location",
      stepIndex: 0,
    });

    view.unmount();

    expect(trackWizardAbandonedMock).toHaveBeenCalledWith(
      expect.objectContaining({
        wizardKey: "workspace_setup",
        lastStepKey: "location",
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
});
