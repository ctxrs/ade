import type { SetStateAction } from "react";
import type { SessionTitlingMode } from "../WorkspaceSetupPage.logic";
import type { WizardRoutePlan, WizardStepKey } from "./wizardFlow";
import { parseUserHost } from "./remoteProfiles";

export type ImportRepoStatus = "idle" | "checking" | "ok" | "error";

export type WorkspaceSetupTargetDraft = {
  remoteHostInput: string;
  remotePortInput: string;
  remoteDataDirInput: string;
};

export type WorkspaceSetupEffectiveTarget =
  | {
      kind: "local";
      targetKey: "local";
    }
  | {
      kind: "remote";
      targetKey: string;
      hostInput: string;
      host: string;
      user: string | null;
      portInput: string;
      port: number;
      dataDirInput: string;
      dataDir: string | null;
    };

export type WorkspaceSetupDraftState = {
  targetDraft: WorkspaceSetupTargetDraft;
  sourcePath: string;
  repoUrl: string;
  repoBranch: string;
  workspaceName: string;
  networkAllowlist: string;
  setupHook: string;
  targetBranch: string;
  targetBranchTouched: boolean;
  verifyCommand: string;
  pushOnSuccess: boolean;
  pushRemote: string;
  pushBranch: string;
  pushBranchTouched: boolean;
  createError: string | null;
  importRepoStatus: ImportRepoStatus;
  importRepoNote: string | null;
};

export const createInitialWorkspaceSetupTargetDraft = (): WorkspaceSetupTargetDraft => ({
  remoteHostInput: "",
  remotePortInput: "4399",
  remoteDataDirInput: "",
});

export const createInitialWorkspaceSetupDraftState = (): WorkspaceSetupDraftState => ({
  targetDraft: createInitialWorkspaceSetupTargetDraft(),
  sourcePath: "",
  repoUrl: "",
  repoBranch: "",
  workspaceName: "",
  networkAllowlist: "",
  setupHook: "",
  targetBranch: "main",
  targetBranchTouched: false,
  verifyCommand: "",
  pushOnSuccess: false,
  pushRemote: "origin",
  pushBranch: "main",
  pushBranchTouched: false,
  createError: null,
  importRepoStatus: "idle",
  importRepoNote: null,
});

export const parseWorkspaceSetupRemotePort = (raw: string): number | null => {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const value = Number(trimmed);
  if (!Number.isFinite(value)) return null;
  const port = Math.trunc(value);
  if (port < 1 || port > 65535) return null;
  return port;
};

export const deriveWorkspaceSetupEffectiveTarget = (
  location: string | null | undefined,
  targetDraft: WorkspaceSetupTargetDraft,
): WorkspaceSetupEffectiveTarget | null => {
  if (location === "local") {
    return {
      kind: "local",
      targetKey: "local",
    };
  }
  if (location !== "remote") {
    return null;
  }

  const parsedRemote = parseUserHost(targetDraft.remoteHostInput);
  const parsedRemotePort = parseWorkspaceSetupRemotePort(targetDraft.remotePortInput);
  if (!parsedRemote?.host || parsedRemotePort === null) {
    return null;
  }

  const normalizedDataDir = targetDraft.remoteDataDirInput.trim();
  return {
    kind: "remote",
    targetKey: `ssh:${parsedRemote.user ?? ""}@${parsedRemote.host}:${parsedRemotePort}:${normalizedDataDir}`,
    hostInput: targetDraft.remoteHostInput,
    host: parsedRemote.host,
    user: parsedRemote.user ?? null,
    portInput: targetDraft.remotePortInput,
    port: parsedRemotePort,
    dataDirInput: targetDraft.remoteDataDirInput,
    dataDir: normalizedDataDir || null,
  };
};

export type WorkspaceSetupDraftSetter<T> = (value: SetStateAction<T>) => void;

export type WorkspaceSetupProvisioningSnapshot = {
  targetKey: string;
  containerSelection: string;
  authImportCandidateCount: number;
  missingHarnessCount: number;
  titlingRequired: boolean;
  titlingMode: SessionTitlingMode;
};

export type RoutePlanInsertionStep = Extract<
  WizardStepKey,
  "harness-downloads" | "auth-import" | "session-titling"
>;

export type EnsureOnboardingAfterDaemonConnectResult = {
  routePlan: WizardRoutePlan;
  insertionStep: RoutePlanInsertionStep | null;
};
