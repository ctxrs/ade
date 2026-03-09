import type { SetStateAction } from "react";
import type { SessionTitlingMode } from "../WorkspaceSetupPage.logic";
import type { WizardRoutePlan, WizardStepKey } from "./wizardFlow";

export type ImportRepoStatus = "idle" | "checking" | "ok" | "error";

export type WorkspaceSetupDraftState = {
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

export const createInitialWorkspaceSetupDraftState = (): WorkspaceSetupDraftState => ({
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

