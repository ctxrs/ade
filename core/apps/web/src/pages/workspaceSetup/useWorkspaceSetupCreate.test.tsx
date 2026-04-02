import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useWorkspaceSetupCreate } from "./useWorkspaceSetupCreate";
import type { WorkspaceSetupCreateIntent } from "./createHandoff";
import { deriveWorkspaceSetupEffectiveTarget, type RoutePlanInsertionStep } from "./workflowTypes";

const apiMocks = vi.hoisted(() => ({
  createWorkspace: vi.fn(),
  deleteWorkspace: vi.fn(),
  idToString: vi.fn((id: string | number) => String(id)),
  listWorkspaces: vi.fn(),
  repoClone: vi.fn(),
  repoInit: vi.fn(),
  repoStatus: vi.fn(),
  repoStagingPath: vi.fn(),
  repoValidateDestination: vi.fn(),
  updateWorkspaceExecutionConfig: vi.fn(),
  updateWorkspaceMergeQueueConfig: vi.fn(),
  updateWorkspaceWorktreeBootstrapConfig: vi.fn(),
}));

const desktopMocks = vi.hoisted(() => ({
  desktopConnectLocal: vi.fn(),
  desktopConnectSsh: vi.fn(),
  desktopPickFolder: vi.fn(),
}));

const launchHandoffMocks = vi.hoisted(() => ({
  startWorkspaceSetupLaunchHandoff: vi.fn(),
  waitForLaunchHandoffTerminal: vi.fn(),
}));

const analyticsMocks = vi.hoisted(() => ({
  trackWorkspaceLaunchCompleted: vi.fn(),
}));

const launcherRecentsMocks = vi.hoisted(() => ({
  upsertLauncherRecent: vi.fn(),
}));

vi.mock("../../api/client", () => ({
  createWorkspace: apiMocks.createWorkspace,
  deleteWorkspace: apiMocks.deleteWorkspace,
  idToString: apiMocks.idToString,
  listWorkspaces: apiMocks.listWorkspaces,
  repoClone: apiMocks.repoClone,
  repoInit: apiMocks.repoInit,
  repoStatus: apiMocks.repoStatus,
  repoStagingPath: apiMocks.repoStagingPath,
  repoValidateDestination: apiMocks.repoValidateDestination,
  updateWorkspaceExecutionConfig: apiMocks.updateWorkspaceExecutionConfig,
  updateWorkspaceMergeQueueConfig: apiMocks.updateWorkspaceMergeQueueConfig,
  updateWorkspaceWorktreeBootstrapConfig: apiMocks.updateWorkspaceWorktreeBootstrapConfig,
}));

vi.mock("../../utils/desktop", () => ({
  desktopConnectLocal: desktopMocks.desktopConnectLocal,
  desktopConnectSsh: desktopMocks.desktopConnectSsh,
  desktopPickFolder: desktopMocks.desktopPickFolder,
}));

vi.mock("./launchHandoff", () => ({
  mergeWorkspaceSetupLaunchLogs: (prev: unknown[], next: unknown[]) => [...prev, ...next],
  startWorkspaceSetupLaunchHandoff: launchHandoffMocks.startWorkspaceSetupLaunchHandoff,
  waitForLaunchHandoffTerminal: launchHandoffMocks.waitForLaunchHandoffTerminal,
}));

vi.mock("../../utils/analytics", () => ({
  trackWorkspaceLaunchCompleted: analyticsMocks.trackWorkspaceLaunchCompleted,
}));

vi.mock("../../state/launcherRecentsStore", () => ({
  upsertLauncherRecent: launcherRecentsMocks.upsertLauncherRecent,
}));

const remoteEffectiveTarget = deriveWorkspaceSetupEffectiveTarget("remote", {
  remoteHostInput: "ctxfixture@127.0.0.1",
  remotePortInput: "44099",
  remoteDataDirInput: "/tmp/ctx-remote",
});

if (!remoteEffectiveTarget) {
  throw new Error("expected a remote effective target for create hook tests");
}

const buildIntent = (): WorkspaceSetupCreateIntent => ({
  selections: {
    location: "remote",
    container: "sandbox",
    source: "new",
    network: "full",
  },
  sourcePath: "/remote/new-sandbox",
  repoUrl: "",
  repoBranch: "",
  workspaceName: "remote-sandbox",
  networkAllowlist: "",
  useSandboxStaging: false,
  importRepoStatus: "idle",
  importRepoNote: null,
  targetBranch: "",
  verifyCommand: "",
  mergeQueueSkipped: true,
  pushOnSuccess: false,
  pushRemote: "origin",
  pushBranch: "main",
  setupHook: "",
  titlingStepVisible: false,
  titlingMode: "skip" as const,
  titlingRemoteValid: false,
  titlingPersistError: null,
});

const baseLaunchSnapshot = {
  job_id: "launch-1",
  workspace_id: "ws-1",
  kind: "workspace_launch" as const,
  state: "ready" as const,
  created_at: "2026-04-01T00:00:00.000Z",
  updated_at: "2026-04-01T00:00:00.000Z",
  started_at: "2026-04-01T00:00:00.000Z",
  finished_at: "2026-04-01T00:00:01.000Z",
  current_phase: "complete",
  current_step_label: "Ready",
  phases: [],
  logs: [],
  error: null,
};

const renderCreateHook = (insertionStep: RoutePlanInsertionStep | null) => {
  const onOnboardingInsertionRequested = vi.fn();
  const navigate = vi.fn();
  const applyConnection = vi.fn();
  const rememberRemoteProfile = vi.fn();
  const setCreateError = vi.fn();
  const wizardCompletedRef = { current: false };
  const trackWizardCompleted = vi.fn();

  const hook = renderHook(() =>
    useWorkspaceSetupCreate({
      currentStepKey: "confirm",
      intent: buildIntent(),
      ensureTitlingPersistedForCurrentTarget: vi.fn(async () => true),
      setSourcePath: vi.fn(),
      setImportRepoStatus: vi.fn(),
      setImportRepoNote: vi.fn(),
      onOnboardingInsertionRequested,
      onCreateErrorStep: vi.fn(),
      navigate,
      wizardCompletedRef,
      wizardKey: "wizard-remote-sandbox",
      trackWizardCompleted,
      desktopApp: true,
      remotePasswordOnce: null,
      effectiveTarget: remoteEffectiveTarget,
      connectDaemonForImport: vi.fn(async () => {}),
      ensureOnboardingAfterDaemonConnect: vi.fn(async () => ({ insertionStep })),
      waitForDaemonReady: vi.fn(async () => {}),
      applyConnection,
      rememberRemoteProfile,
      setCreateError,
    }),
  );

  return {
    hook,
    onOnboardingInsertionRequested,
    navigate,
    applyConnection,
    rememberRemoteProfile,
    setCreateError,
    wizardCompletedRef,
    trackWizardCompleted,
  };
};

describe("useWorkspaceSetupCreate", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    apiMocks.listWorkspaces.mockResolvedValue([]);
    apiMocks.repoInit.mockResolvedValue({ path: "/remote/new-sandbox" });
    apiMocks.createWorkspace.mockResolvedValue({ id: "ws-1" });
    apiMocks.updateWorkspaceExecutionConfig.mockResolvedValue(undefined);
    desktopMocks.desktopConnectSsh.mockResolvedValue({
      kind: "ssh",
      base_url: "http://127.0.0.1:4399",
      token: "token",
    });
    launchHandoffMocks.startWorkspaceSetupLaunchHandoff.mockResolvedValue(baseLaunchSnapshot);
    launchHandoffMocks.waitForLaunchHandoffTerminal.mockResolvedValue(undefined);
    launcherRecentsMocks.upsertLauncherRecent.mockResolvedValue(undefined);
  });

  it("continues remote create when onboarding refresh discovers optional harness downloads", async () => {
    const {
      hook,
      onOnboardingInsertionRequested,
      navigate,
      wizardCompletedRef,
      trackWizardCompleted,
    } = renderCreateHook("harness-downloads");

    await act(async () => {
      await hook.result.current.onCreate();
    });

    expect(onOnboardingInsertionRequested).not.toHaveBeenCalled();
    expect(apiMocks.createWorkspace).toHaveBeenCalledWith(
      "/remote/new-sandbox",
      "remote-sandbox",
      "remote",
      "wizard",
    );
    expect(apiMocks.updateWorkspaceExecutionConfig).toHaveBeenCalledWith("ws-1", {
      environment: "sandbox",
      network_mode: "all",
      allowlist: null,
    });
    expect(desktopMocks.desktopConnectSsh).toHaveBeenCalledWith({
      host: "127.0.0.1",
      user: "ctxfixture",
      password_once: null,
      remote_port: 44099,
      start_remote: true,
      remote_data_dir: "/tmp/ctx-remote",
    });
    expect(navigate).toHaveBeenCalledWith("/workspaces/ws-1", { replace: true });
    expect(wizardCompletedRef.current).toBe(true);
    expect(trackWizardCompleted).toHaveBeenCalledWith({
      wizardKey: "wizard-remote-sandbox",
      workspaceKind: "remote",
    });
  });

  it("still routes back when remote create discovers a blocking titling step", async () => {
    const { hook, onOnboardingInsertionRequested, navigate } = renderCreateHook("session-titling");

    await act(async () => {
      await hook.result.current.onCreate();
    });

    expect(onOnboardingInsertionRequested).toHaveBeenCalledWith("session-titling");
    expect(apiMocks.createWorkspace).not.toHaveBeenCalled();
    expect(navigate).not.toHaveBeenCalled();
  });
});
