import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import WorkspaceSetupPage from "./WorkspaceSetupPage";
import {
  buildExecutionLaunchWsUrl,
  createWorkspace,
  getInstall,
  getExecutionLaunchStatus,
  getHealth,
  getSettings,
  getTitleGenerationLocalStatus,
  importProviderAuthCandidates,
  installTitleGenerationLocal,
  listProviderAuthImportCandidates,
  listWorkspaces,
  repoClone,
  repoInit,
  repoStagingPath,
  repoStatus,
  startExecutionLaunch,
  updateSettings,
  updateWorkspaceExecutionConfig,
  updateWorkspaceMergeQueueConfig,
  updateWorkspaceWorktreeBootstrapConfig,
} from "../api/client";
import { desktopConnectLocal, desktopListSshHosts, desktopTestSsh, isDesktopApp } from "../utils/desktop";

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
    getTitleGenerationLocalStatus: vi.fn(),
    importProviderAuthCandidates: vi.fn(),
    installTitleGenerationLocal: vi.fn(),
    listProviderAuthImportCandidates: vi.fn(),
    listWorkspaces: vi.fn(),
    repoClone: vi.fn(),
    repoInit: vi.fn(),
    repoStatus: vi.fn(),
    repoStagingPath: vi.fn(),
    startExecutionLaunch: vi.fn(),
    updateSettings: vi.fn(),
    updateWorkspaceExecutionConfig: vi.fn(),
    updateWorkspaceMergeQueueConfig: vi.fn(),
    updateWorkspaceWorktreeBootstrapConfig: vi.fn(),
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

const renderPage = () =>
  render(
    <MemoryRouter initialEntries={["/workspace-setup"]}>
      <WorkspaceSetupPage />
    </MemoryRouter>,
  );

const getWizardShell = (): HTMLElement => screen.getByTestId("workspace-setup");

const wizardStepKey = (): string => getWizardShell().getAttribute("data-step-key") ?? "";

describe("WorkspaceSetupPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(buildExecutionLaunchWsUrl).mockReturnValue("ws://127.0.0.1:1/launch");
    vi.mocked(isDesktopApp).mockReturnValue(false);
    vi.mocked(listProviderAuthImportCandidates).mockResolvedValue({ candidates: [] });
    vi.mocked(listWorkspaces).mockResolvedValue([]);
    vi.mocked(getHealth).mockResolvedValue({
      daemon_version: "0.0.0-test",
      compatibility: { desktop_exact_version: "0.0.0-test", mobile_api_min: 1, mobile_api_max: 1 },
    } as never);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);
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
    vi.mocked(desktopListSshHosts).mockResolvedValue([]);
    vi.mocked(desktopTestSsh).mockResolvedValue();
  });

  it("requires location selection before allowing next and advances for local selection", async () => {
    renderPage();
    await screen.findByTestId("workspace-setup");
    expect(wizardStepKey()).toBe("location");

    const nextButton = screen.getByTestId("wizard-next");
    expect(nextButton).toBeDisabled();

    fireEvent.click(screen.getByTestId("wizard-option-location-local"));
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });
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

  it("requires absolute remote ctx binary path for remote flow in desktop mode", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    renderPage();
    await screen.findByTestId("workspace-setup");

    fireEvent.click(screen.getByTestId("wizard-option-location-remote"));
    const hostInput = await screen.findByTestId("wizard-remote-host");
    fireEvent.change(hostInput, { target: { value: "devbox.example" } });

    const nextButton = screen.getByTestId("wizard-next");
    fireEvent.click(nextButton);
    expect(await screen.findByText("Remote ctx binary path is required.")).toBeInTheDocument();
    expect(wizardStepKey()).toBe("location");

    const ctxBinInput = await screen.findByTestId("wizard-remote-ctx-bin");
    fireEvent.change(ctxBinInput, { target: { value: "ctx" } });
    fireEvent.click(nextButton);
    expect(
      await screen.findByText(
        "Remote ctx binary path must be absolute (for example /opt/ctx/bin/ctx).",
      ),
    ).toBeInTheDocument();
    expect(desktopTestSsh).not.toHaveBeenCalled();
  });

  it("renders import auth rows with parsed harnesses preselected", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(listProviderAuthImportCandidates).mockResolvedValue({
      candidates: [
        {
          id: "cand-claude",
          provider_id: "claude-crp",
          provider_label: "Claude Code",
          kind: "auth_file",
          path: "/Users/example-user/.claude.json",
          signal_strength: "strong",
          confidence: "medium",
          parse_status: "parsed",
        },
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
    fireEvent.click(screen.getByTestId("wizard-option-location-local"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("auth-import");
    });

    const claudeCheckbox = screen.getByRole("checkbox", { name: /claude code/i }) as HTMLInputElement;
    const codexCheckbox = screen.getByRole("checkbox", { name: /codex/i }) as HTMLInputElement;
    const cursorCheckbox = screen.getByRole("checkbox", { name: /cursor/i }) as HTMLInputElement;

    expect(claudeCheckbox.checked).toBe(true);
    expect(codexCheckbox.checked).toBe(true);
    expect(cursorCheckbox.checked).toBe(false);
    expect(cursorCheckbox.disabled).toBe(true);
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
    fireEvent.click(screen.getByTestId("wizard-option-location-local"));

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
    fireEvent.click(screen.getByTestId("wizard-option-location-local"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("auth-import");
    });

    fireEvent.click(screen.getByTestId("wizard-next"));
    await waitFor(() => {
      expect(importProviderAuthCandidates).toHaveBeenCalled();
    });
    expect(getSettings).toHaveBeenCalledTimes(1);

    await waitFor(() => {
      expect(wizardStepKey()).toBe("session-titling");
    });
    expect(getSettings).toHaveBeenCalledTimes(1);
  });

  it("shows session titling step when selected daemon is not configured", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    fireEvent.click(screen.getByTestId("wizard-option-location-local"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("session-titling");
    });
    expect(screen.getByTestId("wizard-titling-mode-remote")).toBeInTheDocument();
    expect(screen.getByTestId("wizard-titling-mode-local")).toBeInTheDocument();
  });

  it("masks session titling remote API key input", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    fireEvent.click(screen.getByTestId("wizard-option-location-local"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("session-titling");
    });
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
    fireEvent.click(screen.getByTestId("wizard-option-location-local"));

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
    fireEvent.click(screen.getByTestId("wizard-option-location-local"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("session-titling");
    });
    fireEvent.click(screen.getByTestId("wizard-titling-mode-remote"));
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
      expect(wizardStepKey()).toBe("container");
    });
  });

  it("allows advancing with local titling mode even when local install is not ready", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);
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

    renderPage();
    await screen.findByTestId("workspace-setup");
    fireEvent.click(screen.getByTestId("wizard-option-location-local"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("session-titling");
    });
    fireEvent.click(screen.getByTestId("wizard-titling-mode-local"));
    fireEvent.click(screen.getByTestId("wizard-next"));

    await waitFor(() => {
      expect(updateSettings).toHaveBeenCalledWith(expect.objectContaining({
        title_generation: expect.objectContaining({
          mode: "local",
        }),
      }));
    });
    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });
  });

  it("does not loop local status fetches after a local-status error", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);
    vi.mocked(getTitleGenerationLocalStatus).mockRejectedValue(new Error("status unavailable"));

    renderPage();
    await screen.findByTestId("workspace-setup");
    fireEvent.click(screen.getByTestId("wizard-option-location-local"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("session-titling");
    });
    fireEvent.click(screen.getByTestId("wizard-titling-mode-local"));

    await waitFor(() => {
      expect(getTitleGenerationLocalStatus).toHaveBeenCalledTimes(2);
    });
    const callsAfterInitial = vi.mocked(getTitleGenerationLocalStatus).mock.calls.length;
    await new Promise((resolve) => window.setTimeout(resolve, 80));
    expect(vi.mocked(getTitleGenerationLocalStatus).mock.calls.length).toBe(callsAfterInitial);
  });

  it("supports skip path without writing session titling settings", async () => {
    vi.mocked(isDesktopApp).mockReturnValue(true);
    vi.mocked(getSettings).mockResolvedValue({ title_generation: null } as never);

    renderPage();
    await screen.findByTestId("workspace-setup");
    fireEvent.click(screen.getByTestId("wizard-option-location-local"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("session-titling");
    });
    fireEvent.click(screen.getByTestId("wizard-titling-skip"));

    await waitFor(() => {
      expect(wizardStepKey()).toBe("container");
    });
    expect(updateSettings).not.toHaveBeenCalled();
  });
});
