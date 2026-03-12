import type { Dispatch, MutableRefObject, SetStateAction } from "react";
import { Link } from "react-router-dom";
import { ChevronRight, Info, X } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import LauncherBrand from "../../components/LauncherBrand";
import type {
  ExecutionLaunchSnapshot,
  InstallTarget,
  ProviderAuthImportCandidate,
} from "../../api/client";
import type { DesktopSshPathEntry } from "../../utils/desktop";
import type { SessionTitlingMode } from "../WorkspaceSetupPage.logic";
import {
  AuthImportStepPanel,
  HarnessDownloadsStepPanel,
} from "./WorkspaceSetupPanels";
import type { WorkspaceSetupLaunchLogLine } from "./launchProgress";
import type {
  HarnessInstallProviderRow,
  HarnessInstallRowState,
  ImportInitDialogState,
  RemoteStatus,
  SshSuggestion,
  WizardStep,
} from "./wizardTypes";
import type { WizardStepKey } from "./wizardFlow";

type WorkspaceSetupPageViewProps = {
  importInitDialog: ImportInitDialogState | null;
  resolveImportInitDialog: (confirmed: boolean) => void;
  infoStep: WizardStep | null;
  openInfoKey: string | null;
  setOpenInfoKey: Dispatch<SetStateAction<string | null>>;
  step: WizardStep;
  steps: WizardStep[];
  stepIndex: number;
  selections: Record<string, string>;
  createError: string | null;
  setCreateError: Dispatch<SetStateAction<string | null>>;
  showLaunchPanel: boolean;
  launchSnapshot: ExecutionLaunchSnapshot | null;
  currentLaunchStepLabel: string;
  currentLaunchElapsed: string;
  currentLaunchEtaLabel: string;
  launchCopyLabel: string;
  onCopyLaunchDiagnostics: () => void;
  launchLogs: WorkspaceSetupLaunchLogLine[];
  onSelectOption: (stepKey: string, optionId: string) => void;
  containerAdvancedOpen: boolean;
  setContainerAdvancedOpen: Dispatch<SetStateAction<boolean>>;
  networkAllowlist: string;
  setNetworkAllowlist: Dispatch<SetStateAction<string>>;
  remoteHostInput: string;
  onRemoteInputChange: (value: string) => void;
  remotePasswordPromptVisible: boolean;
  remotePasswordInput: string;
  setRemotePasswordInput: (value: string) => void;
  remoteStatus: RemoteStatus;
  setRemoteStatus: (status: RemoteStatus) => void;
  remoteError: string | null;
  setRemoteError: (value: string | null) => void;
  sshSuggestions: SshSuggestion[];
  authImportBusy: boolean;
  authImportError: string | null;
  authImportCandidates: ProviderAuthImportCandidate[];
  authImportSelected: Record<string, boolean>;
  setAuthImportSelected: Dispatch<SetStateAction<Record<string, boolean>>>;
  harnessByProviderId: Map<string, { logoSrc?: string; invertInDark?: boolean; invertInLight?: boolean }>;
  onSkipAuthImport: () => void;
  harnessInstallBusy: boolean;
  harnessInstallError: string | null;
  selectedHarnessRunningCount: number;
  selectedHarnessBlockedCount: number;
  harnessInstallCandidates: HarnessInstallProviderRow[];
  harnessDownloadsCanScroll: boolean;
  harnessDownloadsAtBottom: boolean;
  harnessDownloadsScrollRef: MutableRefObject<HTMLDivElement | null>;
  updateHarnessDownloadsScrollState: () => void;
  harnessInstallSelected: Record<string, boolean>;
  setHarnessInstallSelected: Dispatch<SetStateAction<Record<string, boolean>>>;
  harnessInstallRows: Record<string, HarnessInstallRowState>;
  selectedHarnessInstallTarget: InstallTarget;
  cancelHarnessInstall: (providerId: string) => void;
  onSkipHarnessDownloads: () => void;
  titlingProbeBusy: boolean;
  titlingProbeError: string | null;
  titlingPersistError: string | null;
  titlingStatusError: string | null;
  titlingMode: SessionTitlingMode;
  setTitlingMode: Dispatch<SetStateAction<SessionTitlingMode>>;
  titlingLocalInstallBusy: boolean;
  titlingPersistBusy: boolean;
  onSelectTitlingLocal: () => void;
  titlingLocalStatus: { ready?: boolean } | null;
  titlingLocalInstall: { state: string; pct: number | null; error?: string } | null;
  titlingRemoteBaseUrl: string;
  setTitlingRemoteBaseUrl: Dispatch<SetStateAction<string>>;
  titlingRemoteApiKey: string;
  setTitlingRemoteApiKey: Dispatch<SetStateAction<string>>;
  titlingRemoteModel: string;
  setTitlingRemoteModel: Dispatch<SetStateAction<string>>;
  titlingRemoteAdvancedOpen: boolean;
  setTitlingRemoteAdvancedOpen: Dispatch<SetStateAction<boolean>>;
  titlingRemoteUseJson: boolean;
  setTitlingRemoteUseJson: Dispatch<SetStateAction<boolean>>;
  invalidateTitlingPersisted: () => void;
  onSkipTitling: () => void;
  needsSourcePath: boolean;
  sourcePath: string;
  setSourcePath: Dispatch<SetStateAction<string>>;
  onPickLocalFolder: () => void;
  importRepoStatus: "idle" | "checking" | "ok" | "error";
  importRepoNote: string | null;
  remotePathSuggestions: DesktopSshPathEntry[];
  remotePathStatus: "idle" | "loading" | "error";
  remotePathError: string | null;
  repoUrl: string;
  setRepoUrl: Dispatch<SetStateAction<string>>;
  repoBranch: string;
  setRepoBranch: Dispatch<SetStateAction<string>>;
  useDiskIsolatedStaging: boolean;
  setupHook: string;
  setSetupHook: Dispatch<SetStateAction<string>>;
  workspaceName: string;
  setWorkspaceName: Dispatch<SetStateAction<string>>;
  mergeQueueSkipped: boolean;
  targetBranch: string;
  setTargetBranch: Dispatch<SetStateAction<string>>;
  setTargetBranchTouched: Dispatch<SetStateAction<boolean>>;
  verifyCommand: string;
  setVerifyCommand: Dispatch<SetStateAction<string>>;
  mergeAdvancedOpen: boolean;
  setMergeAdvancedOpen: Dispatch<SetStateAction<boolean>>;
  pushOnSuccess: boolean;
  setPushOnSuccess: Dispatch<SetStateAction<boolean>>;
  pushRemote: string;
  setPushRemote: Dispatch<SetStateAction<string>>;
  pushBranch: string;
  setPushBranch: Dispatch<SetStateAction<string>>;
  setPushBranchTouched: Dispatch<SetStateAction<boolean>>;
  enableMergeQueueIfSkipped: () => void;
  onMergeSkip: () => void;
  harnessSummaryValue: string;
  titlingSummaryValue: string;
  sourceStepComplete: boolean;
  titlingRemoteValid: boolean;
  hasRemoteHost: boolean;
  goToStepKey: (key: WizardStepKey) => void;
  isFirst: boolean;
  isLast: boolean;
  canAdvance: boolean;
  creating: boolean;
  onCreate: () => void;
  onNext: () => void;
  goRelativeStep: (delta: number) => void;
  createButtonLabel: string;
  nextButtonLabel: string;
};

export function WorkspaceSetupPageView({
  importInitDialog,
  resolveImportInitDialog,
  infoStep,
  openInfoKey,
  setOpenInfoKey,
  step,
  steps,
  stepIndex,
  selections,
  createError,
  setCreateError,
  showLaunchPanel,
  launchSnapshot,
  currentLaunchStepLabel,
  currentLaunchElapsed,
  currentLaunchEtaLabel,
  launchCopyLabel,
  onCopyLaunchDiagnostics,
  launchLogs,
  onSelectOption,
  containerAdvancedOpen,
  setContainerAdvancedOpen,
  networkAllowlist,
  setNetworkAllowlist,
  remoteHostInput,
  onRemoteInputChange,
  remotePasswordPromptVisible,
  remotePasswordInput,
  setRemotePasswordInput,
  remoteStatus,
  setRemoteStatus,
  remoteError,
  setRemoteError,
  sshSuggestions,
  authImportBusy,
  authImportError,
  authImportCandidates,
  authImportSelected,
  setAuthImportSelected,
  harnessByProviderId,
  onSkipAuthImport,
  harnessInstallBusy,
  harnessInstallError,
  selectedHarnessRunningCount,
  selectedHarnessBlockedCount,
  harnessInstallCandidates,
  harnessDownloadsCanScroll,
  harnessDownloadsAtBottom,
  harnessDownloadsScrollRef,
  updateHarnessDownloadsScrollState,
  harnessInstallSelected,
  setHarnessInstallSelected,
  harnessInstallRows,
  selectedHarnessInstallTarget,
  cancelHarnessInstall,
  onSkipHarnessDownloads,
  titlingProbeBusy,
  titlingProbeError,
  titlingPersistError,
  titlingStatusError,
  titlingMode,
  setTitlingMode,
  titlingLocalInstallBusy,
  titlingPersistBusy,
  onSelectTitlingLocal,
  titlingLocalStatus,
  titlingLocalInstall,
  titlingRemoteBaseUrl,
  setTitlingRemoteBaseUrl,
  titlingRemoteApiKey,
  setTitlingRemoteApiKey,
  titlingRemoteModel,
  setTitlingRemoteModel,
  titlingRemoteAdvancedOpen,
  setTitlingRemoteAdvancedOpen,
  titlingRemoteUseJson,
  setTitlingRemoteUseJson,
  invalidateTitlingPersisted,
  onSkipTitling,
  needsSourcePath,
  sourcePath,
  setSourcePath,
  onPickLocalFolder,
  importRepoStatus,
  importRepoNote,
  remotePathSuggestions,
  remotePathStatus,
  remotePathError,
  repoUrl,
  setRepoUrl,
  repoBranch,
  setRepoBranch,
  useDiskIsolatedStaging,
  setupHook,
  setSetupHook,
  workspaceName,
  setWorkspaceName,
  mergeQueueSkipped,
  targetBranch,
  setTargetBranch,
  setTargetBranchTouched,
  verifyCommand,
  setVerifyCommand,
  mergeAdvancedOpen,
  setMergeAdvancedOpen,
  pushOnSuccess,
  setPushOnSuccess,
  pushRemote,
  setPushRemote,
  pushBranch,
  setPushBranch,
  setPushBranchTouched,
  enableMergeQueueIfSkipped,
  onMergeSkip,
  harnessSummaryValue,
  titlingSummaryValue,
  sourceStepComplete,
  titlingRemoteValid,
  hasRemoteHost,
  goToStepKey,
  isFirst,
  isLast,
  canAdvance,
  creating,
  onCreate,
  onNext,
  goRelativeStep,
  createButtonLabel,
  nextButtonLabel,
}: WorkspaceSetupPageViewProps) {
  return (
    <div className="launcher-shell launcher-shell--crt">
      <LauncherBrand fullScreen>
        <div className="wizard-panel" data-testid="workspace-setup" data-step-key={step.key}>
          {importInitDialog && (
            <div
              className="wizard-modal-backdrop"
              role="dialog"
              aria-modal="true"
              aria-label="Initialize Git repo"
              data-testid="wizard-import-init-modal"
              onClick={() => resolveImportInitDialog(false)}
            >
              <div className="wizard-modal" onClick={(event) => event.stopPropagation()}>
                <div className="wizard-modal-header">
                  <div className="wizard-modal-title">Initialize Git repo in this folder?</div>
                  <button
                    type="button"
                    className="wizard-modal-close"
                    aria-label="Close"
                    onClick={() => resolveImportInitDialog(false)}
                  >
                    <X size={16} aria-hidden="true" />
                  </button>
                </div>
                <div className="wizard-modal-body">
                  <div className="wizard-modal-copy">
                    The selected folder is not currently a repository.
                  </div>
                  <div className="wizard-modal-path">
                    <code>{importInitDialog.path}</code>
                  </div>
                  <div className="wizard-modal-note">
                    This will run <code>git init</code> and create one empty initial commit. Existing files are not staged or committed.
                  </div>
                  <div className="wizard-modal-actions">
                    <button
                      type="button"
                      className="wizard-secondary"
                      data-testid="wizard-import-init-cancel"
                      onClick={() => resolveImportInitDialog(false)}
                    >
                      Cancel
                    </button>
                    <button
                      type="button"
                      className="wizard-primary"
                      data-testid="wizard-import-init-confirm"
                      onClick={() => resolveImportInitDialog(true)}
                    >
                      Initialize Git repo here
                    </button>
                  </div>
                </div>
              </div>
            </div>
          )}
          {infoStep?.info && (
            <div
              className="wizard-modal-backdrop"
              role="dialog"
              aria-modal="true"
              aria-label={`${infoStep.title} info`}
              onClick={() => setOpenInfoKey(null)}
            >
              <div className="wizard-modal" onClick={(event) => event.stopPropagation()}>
                <div className="wizard-modal-header">
                  <div className="wizard-modal-title">{infoStep.title}</div>
                  <button
                    type="button"
                    className="wizard-modal-close"
                    aria-label="Close"
                    onClick={() => setOpenInfoKey(null)}
                  >
                    <X size={16} aria-hidden="true" />
                  </button>
                </div>
                <div className="wizard-modal-body">
                  <div className="wizard-markdown">
                    <ReactMarkdown remarkPlugins={[remarkGfm]}>
                      {infoStep.info}
                    </ReactMarkdown>
                  </div>
                </div>
              </div>
            </div>
          )}
          <div className="wizard-steps">
            <div className="wizard-step" data-testid="wizard-step" data-step-key={step.key}>
              <div className="wizard-step-header">
                <div className="wizard-step-title-row">
                  <div className="wizard-step-title">{step.title}</div>
                  {step.info && (
                    <button
                      type="button"
                      className="wizard-info-toggle"
                      onClick={() => setOpenInfoKey((prev) => (prev === step.key ? null : step.key))}
                      aria-label="Info"
                    >
                      <Info size={16} aria-hidden="true" />
                    </button>
                  )}
                </div>
                <div className="wizard-step-note">{step.note}</div>
              </div>
              <div className="wizard-step-body">
                {createError && (
                  <div className="wizard-error">{createError}</div>
                )}
                {showLaunchPanel && launchSnapshot && (
                  <div className="wizard-launch-log-panel" data-testid="wizard-launch-log-panel">
                    <div className="wizard-launch-log-header">
                      <div>
                        <div className="wizard-launch-log-title">Workspace Launch Logs</div>
                        <div className="wizard-launch-log-meta">
                          <span>{currentLaunchStepLabel}</span>
                          <span>{currentLaunchElapsed} elapsed</span>
                          <span>{currentLaunchEtaLabel}</span>
                        </div>
                      </div>
                      <button
                        type="button"
                        className="wizard-input-button"
                        onClick={onCopyLaunchDiagnostics}
                        data-testid="wizard-launch-copy"
                      >
                        {launchCopyLabel}
                      </button>
                    </div>
                    <div className="wizard-launch-log-body">
                      {launchLogs.length === 0 ? (
                        <div className="wizard-note">Waiting for launch logs…</div>
                      ) : (
                        launchLogs.map((line) => (
                          <div key={line.seq} className="wizard-launch-log-line">
                            <span className="wizard-launch-log-ts">{line.timeLabel}</span>
                            <span className="wizard-launch-log-phase">{line.phaseLabel}</span>
                            <span className={`wizard-launch-log-level wizard-launch-log-level--${line.level}`}>{line.level}</span>
                            <span className="wizard-launch-log-msg">{line.message}</span>
                          </div>
                        ))
                      )}
                    </div>
                  </div>
                )}
                {step.options && (
                  <>
                    <div className="wizard-option-grid">
                      {step.options
                        .filter((option) => step.key !== "container" || !option.advanced)
                        .map((option) => {
                          const selected = selections[step.key] === option.id;
                          return (
                            <button
                              key={option.id}
                              type="button"
                              className={`wizard-option${selected ? " is-selected" : ""}`}
                              data-testid={`wizard-option-${step.key}-${option.id}`}
                              onClick={() => onSelectOption(step.key, option.id)}
                              aria-pressed={selected}
                            >
                              <div className="wizard-option-title">
                                <span className="wizard-option-title-text">{option.title}</span>
                                {option.badge && <span className="wizard-option-badge">{option.badge}</span>}
                              </div>
                              <div className="wizard-option-desc">{option.desc}</div>
                            </button>
                          );
                        })}
                    </div>
                    {step.key === "container" && (
                      <button
                        type="button"
                        className="wizard-advanced-link"
                        data-testid="wizard-container-advanced-toggle"
                        onClick={() => setContainerAdvancedOpen((open) => !open)}
                        aria-expanded={containerAdvancedOpen}
                      >
                        <ChevronRight
                          size={14}
                          className={containerAdvancedOpen ? "is-open" : undefined}
                          aria-hidden="true"
                        />
                        Advanced
                      </button>
                    )}
                    {step.key === "container" && containerAdvancedOpen && (
                      <div className="wizard-container-advanced">
                        <div className="wizard-option-grid wizard-option-grid--two">
                          {step.options
                            .filter((option) => Boolean(option.advanced))
                            .map((option) => {
                              const selected = selections[step.key] === option.id;
                              return (
                                <button
                                  key={option.id}
                                  type="button"
                                  className={`wizard-option${selected ? " is-selected" : ""}`}
                                  data-testid={`wizard-option-${step.key}-${option.id}`}
                                  onClick={() => onSelectOption(step.key, option.id)}
                                  aria-pressed={selected}
                                >
                                  <div className="wizard-option-title">
                                    <span className="wizard-option-title-text">{option.title}</span>
                                    {option.badge && <span className="wizard-option-badge">{option.badge}</span>}
                                  </div>
                                  <div className="wizard-option-desc">{option.desc}</div>
                                </button>
                              );
                            })}
                        </div>
                      </div>
                    )}
                  </>
                )}
                {step.key === "network" && selections.network === "allowlist" && (
                  <div className="wizard-input">
                    <label>
                      Allowed hosts (one per line)
                      <textarea
                        data-testid="wizard-network-allowlist"
                        placeholder={"github.com\nregistry.npmjs.org\npypi.org"}
                        value={networkAllowlist}
                        onChange={(event) => {
                          setCreateError(null);
                          setNetworkAllowlist(event.target.value);
                        }}
                        rows={6}
                      />
                    </label>
                  </div>
                )}
                {step.key === "location" && selections.location === "remote" && (
                  <div className="wizard-remote">
                    <div className="wizard-input">
                      <label>
                        Remote host
                        <input
                          data-testid="wizard-remote-host"
                          placeholder="user@host"
                          value={remoteHostInput}
                          onChange={(event) => onRemoteInputChange(event.target.value)}
                        />
                      </label>
                    </div>
                    {remotePasswordPromptVisible ? (
                      <div className="wizard-input">
                        <label>
                          SSH Password
                          <input
                            data-testid="wizard-remote-password-once"
                            type="password"
                            autoComplete="current-password"
                            value={remotePasswordInput}
                            onChange={(event) => {
                              setCreateError(null);
                              setRemotePasswordInput(event.target.value);
                              if (remoteStatus !== "idle") {
                                setRemoteStatus("idle");
                                setRemoteError(null);
                              }
                            }}
                          />
                        </label>
                        <div className="wizard-note">
                          Used to install SSH key auth; never stored
                        </div>
                      </div>
                    ) : null}
                    {sshSuggestions.length > 0 && (
                      <div className="wizard-remote-list">
                        {sshSuggestions.map((entry) => {
                          const label = entry.user ? `${entry.user}@${entry.host}` : entry.host;
                          return (
                            <button
                              key={label}
                              type="button"
                              className="wizard-remote-suggestion"
                              onClick={() => onRemoteInputChange(label)}
                            >
                              {label}
                            </button>
                          );
                        })}
                      </div>
                    )}
                    {remoteStatus === "connecting" && (
                      <div className="wizard-note">Connecting…</div>
                    )}
                    {remoteStatus === "connected" && (
                      <div className="wizard-note">Connection verified.</div>
                    )}
                    {remoteStatus === "error" && remoteError && (
                      <div className="wizard-error">{remoteError}</div>
                    )}
                  </div>
                )}
                {step.key === "auth-import" && (
                  <AuthImportStepPanel
                    busy={authImportBusy}
                    error={authImportError}
                    candidates={authImportCandidates}
                    selected={authImportSelected}
                    setSelected={setAuthImportSelected}
                    harnessByProviderId={harnessByProviderId}
                    onSkip={onSkipAuthImport}
                  />
                )}
                {step.key === "harness-downloads" && (
                  <HarnessDownloadsStepPanel
                    busy={harnessInstallBusy}
                    error={harnessInstallError}
                    selectedRunningCount={selectedHarnessRunningCount}
                    selectedBlockedCount={selectedHarnessBlockedCount}
                    candidates={harnessInstallCandidates}
                    canScroll={harnessDownloadsCanScroll}
                    atBottom={harnessDownloadsAtBottom}
                    scrollRef={harnessDownloadsScrollRef}
                    onScroll={updateHarnessDownloadsScrollState}
                    selected={harnessInstallSelected}
                    setSelected={setHarnessInstallSelected}
                    rows={harnessInstallRows}
                    selectedInstallTarget={selectedHarnessInstallTarget}
                    harnessByProviderId={harnessByProviderId}
                    onCancelInstall={cancelHarnessInstall}
                    onSkip={onSkipHarnessDownloads}
                  />
                )}
                {step.key === "session-titling" && (
                  <div className="wizard-input">
                    {titlingProbeBusy ? (
                      <div className="wizard-note">Checking session titling configuration on this daemon…</div>
                    ) : null}
                    {titlingProbeError ? (
                      <div className="wizard-error">
                        Could not auto-detect titling configuration. You can still configure now or skip. ({titlingProbeError})
                      </div>
                    ) : null}
                    {titlingPersistError ? <div className="wizard-error">{titlingPersistError}</div> : null}
                    {titlingStatusError ? <div className="wizard-error">{titlingStatusError}</div> : null}
                    <div className="wizard-option-grid wizard-option-grid--two">
                      <button
                        type="button"
                        className={`wizard-option${titlingMode === "remote" ? " is-selected" : ""}`}
                        data-testid="wizard-titling-mode-remote"
                        onClick={() => {
                          invalidateTitlingPersisted();
                          setTitlingMode("remote");
                        }}
                        disabled={titlingLocalInstallBusy || titlingPersistBusy}
                        aria-pressed={titlingMode === "remote"}
                      >
                        <div className="wizard-option-title">
                          <span className="wizard-option-title-text">Remote LLM via API Key</span>
                        </div>
                        <div className="wizard-option-desc">
                          Use a cloud endpoint with API key + model for title generation.
                        </div>
                      </button>
                      <button
                        type="button"
                        className={`wizard-option${titlingMode === "local" ? " is-selected" : ""}`}
                        data-testid="wizard-titling-mode-local"
                        onClick={onSelectTitlingLocal}
                        disabled
                        aria-pressed={titlingMode === "local"}
                      >
                        <div className="wizard-option-title">
                          <span className="wizard-option-title-text">Local model</span>
                        </div>
                        <div className="wizard-option-desc">
                          Coming soon: download a small LLM to run locally for generating task titles.
                        </div>
                      </button>
                    </div>
                    {titlingMode === "local" ? (
                      <div className="wizard-note" data-testid="wizard-titling-local-status">
                        {titlingLocalStatus?.ready
                          ? "Local model ready."
                          : titlingLocalInstallBusy
                            ? "Starting local model download…"
                            : titlingLocalInstall?.state === "running"
                              ? `Installing local model${typeof titlingLocalInstall.pct === "number" ? ` (${titlingLocalInstall.pct}%)` : ""}. This continues in background.`
                              : titlingLocalInstall?.state === "cancelled"
                                ? "Local model install cancelled."
                                : titlingLocalInstall?.state === "failed"
                                  ? `Local model install failed${titlingLocalInstall.error ? `: ${titlingLocalInstall.error}` : "."}`
                                  : "Local model is not ready yet. Titles use fallback until install completes."}
                      </div>
                    ) : null}
                    {titlingMode === "remote" && (
                      <div className="wizard-input">
                        <label>
                          Endpoint base URL
                          <input
                            data-testid="wizard-titling-remote-base-url"
                            placeholder="https://api.your-llm-gateway.example/v1"
                            value={titlingRemoteBaseUrl}
                            onChange={(event) => {
                              invalidateTitlingPersisted();
                              setTitlingRemoteBaseUrl(event.target.value);
                            }}
                          />
                        </label>
                        <label>
                          API key
                          <input
                            data-testid="wizard-titling-remote-api-key"
                            placeholder="sk-..."
                            value={titlingRemoteApiKey}
                            type="password"
                            onChange={(event) => {
                              invalidateTitlingPersisted();
                              setTitlingRemoteApiKey(event.target.value);
                            }}
                          />
                        </label>
                        <label>
                          Model
                          <input
                            data-testid="wizard-titling-remote-model"
                            placeholder="model-slug"
                            value={titlingRemoteModel}
                            onChange={(event) => {
                              invalidateTitlingPersisted();
                              setTitlingRemoteModel(event.target.value);
                            }}
                          />
                        </label>
                        <button
                          type="button"
                          className="wizard-advanced-link"
                          data-testid="wizard-titling-remote-advanced-toggle"
                          onClick={() => setTitlingRemoteAdvancedOpen((open) => !open)}
                          aria-expanded={titlingRemoteAdvancedOpen}
                        >
                          <ChevronRight
                            size={14}
                            className={titlingRemoteAdvancedOpen ? "is-open" : undefined}
                            aria-hidden="true"
                          />
                          Advanced
                        </button>
                        {titlingRemoteAdvancedOpen && (
                          <label className="wizard-checkbox">
                            <input
                              data-testid="wizard-titling-remote-use-json"
                              type="checkbox"
                              checked={titlingRemoteUseJson}
                              onChange={(event) => {
                                invalidateTitlingPersisted();
                                setTitlingRemoteUseJson(event.target.checked);
                              }}
                            />
                            Prefer JSON response format
                          </label>
                        )}
                      </div>
                    )}
                    <button
                      type="button"
                      className="wizard-skip wizard-skip--left wizard-skip--below"
                      data-testid="wizard-titling-skip"
                      onClick={onSkipTitling}
                      disabled={titlingPersistBusy || titlingLocalInstallBusy}
                    >
                      Skip for now
                    </button>
                  </div>
                )}
                {step.key === "source" && needsSourcePath && (
                  <div className="wizard-input">
                    <label>
                      {selections.source === "import"
                        ? "Existing folder"
                        : "Destination folder (host)"}
                      <div className="wizard-input-row">
                        <input
                          data-testid="wizard-source-path"
                          placeholder={selections.source === "import" ? "/Users/example-user/project" : "/Users/example-user/projects/"}
                          value={sourcePath}
                          onChange={(event) => {
                            setCreateError(null);
                            setSourcePath(event.target.value);
                          }}
                        />
                        {selections.location === "local" && (
                          <button
                            type="button"
                            className="wizard-input-button"
                            onClick={onPickLocalFolder}
                          >
                            Browse
                          </button>
                        )}
                      </div>
                    </label>
                    {selections.container !== "no-container" && (
                      <div className="wizard-note">
                        This is the project folder on the host. In disk-isolated mode, ctx will copy the workspace into a container-managed filesystem for execution.
                      </div>
                    )}
                    {selections.source === "import" && importRepoStatus !== "idle" && importRepoNote && (
                      <div className={importRepoStatus === "error" ? "wizard-error" : "wizard-note"}>
                        {importRepoNote}
                      </div>
                    )}
                    {selections.location === "remote" && remotePathSuggestions.length > 0 && (
                      <div className="wizard-path-list">
                        {remotePathSuggestions.map((entry) => (
                          <button
                            key={entry.path}
                            type="button"
                            className="wizard-path-suggestion"
                            onClick={() => setSourcePath(`${entry.path}/`)}
                          >
                            {entry.name}
                          </button>
                        ))}
                      </div>
                    )}
                    {selections.location === "remote" && remotePathStatus === "loading" && (
                      <div className="wizard-note">Loading folders…</div>
                    )}
                    {selections.location === "remote" && remotePathStatus === "error" && remotePathError && (
                      <div className="wizard-error">{remotePathError}</div>
                    )}
                  </div>
                )}
                {step.key === "source" && selections.source === "clone" && (
                  <div className="wizard-input">
                    <label>
                      Repo URL
                      <input
                        data-testid="wizard-repo-url"
                        placeholder="https://github.com/org/repo.git"
                        value={repoUrl}
                        onChange={(event) => {
                          setCreateError(null);
                          setRepoUrl(event.target.value);
                        }}
                      />
                    </label>
                    <label>
                      Branch (optional)
                      <input
                        data-testid="wizard-repo-branch"
                        placeholder="main"
                        value={repoBranch}
                        onChange={(event) => {
                          setCreateError(null);
                          setRepoBranch(event.target.value);
                        }}
                      />
                    </label>
                    {useDiskIsolatedStaging && (
                      <div className="wizard-note">
                        Ctx will clone into a managed staging path under <code>~/.ctx/workspaces/staging/</code> by default. Your workspace will live in the container.
                      </div>
                    )}
                    {!useDiskIsolatedStaging && (
                      <div className="wizard-note">
                        Tip: If you enter a folder ending in <code>/</code>, ctx will derive the repo name from the URL.
                      </div>
                    )}
                  </div>
                )}
                {step.key === "setup" && (
                  <div className="wizard-input">
                    <input
                      data-testid="wizard-setup-hook"
                      placeholder="./prepare-worktree.sh"
                      value={setupHook}
                      onChange={(event) => {
                        setCreateError(null);
                        setSetupHook(event.target.value);
                      }}
                    />
                  </div>
                )}
                {step.key === "source" && (selections.source === "new" || selections.source === "import") && (
                  <div className="wizard-input">
                    <label>
                      Workspace name (optional)
                      <input
                        data-testid="wizard-workspace-name"
                        placeholder="workspace"
                        value={workspaceName}
                        onChange={(event) => {
                          setCreateError(null);
                          setWorkspaceName(event.target.value);
                        }}
                      />
                    </label>
                    {selections.source === "new" && useDiskIsolatedStaging && (
                      <div className="wizard-note">
                        Ctx will create the repo in a managed staging path. Your workspace will live in the container.
                      </div>
                    )}
                  </div>
                )}
                {step.key === "merge-queue" && (
                  <div className="wizard-input">
                    <label>
                      Target branch
                      <input
                        data-testid="wizard-merge-target-branch"
                        placeholder="main"
                        value={targetBranch}
                        onChange={(event) => {
                          setCreateError(null);
                          enableMergeQueueIfSkipped();
                          setTargetBranch(event.target.value);
                          setTargetBranchTouched(true);
                        }}
                        disabled={mergeQueueSkipped}
                      />
                    </label>
                    <label>
                      Verification command (optional)
                      <input
                        data-testid="wizard-merge-verify-command"
                        placeholder="./verify.sh"
                        value={verifyCommand}
                        onChange={(event) => {
                          setCreateError(null);
                          enableMergeQueueIfSkipped();
                          setVerifyCommand(event.target.value);
                        }}
                        disabled={mergeQueueSkipped}
                      />
                    </label>
                    <button
                      type="button"
                      className="wizard-advanced-link"
                      data-testid="wizard-merge-advanced-toggle"
                      onClick={() => setMergeAdvancedOpen((open) => !open)}
                      aria-expanded={mergeAdvancedOpen}
                      disabled={mergeQueueSkipped}
                    >
                      <ChevronRight
                        size={14}
                        className={mergeAdvancedOpen ? "is-open" : undefined}
                        aria-hidden="true"
                      />
                      Advanced
                    </button>
                    {mergeAdvancedOpen && (
                      <div className="wizard-advanced-panel">
                        <label className="wizard-checkbox">
                          <input
                            data-testid="wizard-merge-push-on-success"
                            type="checkbox"
                            checked={pushOnSuccess}
                            onChange={(event) => {
                              setCreateError(null);
                              setPushOnSuccess(event.target.checked);
                            }}
                            disabled={mergeQueueSkipped}
                          />
                          Push to remote on success
                        </label>
                        {pushOnSuccess && (
                          <div className="wizard-input">
                            <label>
                              Push remote
                              <input
                                data-testid="wizard-merge-push-remote"
                                placeholder="origin"
                                value={pushRemote}
                                onChange={(event) => {
                                  setCreateError(null);
                                  setPushRemote(event.target.value);
                                }}
                                disabled={mergeQueueSkipped}
                              />
                            </label>
                            <label>
                              Push branch
                              <input
                                data-testid="wizard-merge-push-branch"
                                placeholder={targetBranch || "main"}
                                value={pushBranch}
                                onChange={(event) => {
                                  setCreateError(null);
                                  setPushBranch(event.target.value);
                                  setPushBranchTouched(true);
                                }}
                                disabled={mergeQueueSkipped}
                              />
                            </label>
                          </div>
                        )}
                      </div>
                    )}
                    <button
                      type="button"
                      className="wizard-skip wizard-skip--left wizard-skip--below"
                      data-testid="wizard-merge-skip"
                      onClick={onMergeSkip}
                    >
                      Skip for now
                    </button>
                  </div>
                )}
                {step.key === "confirm" && (
                  <div className="wizard-step-summary">
                    <div className="wizard-summary">
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Location</div>
                        <div className="wizard-summary-v">
                          {selections.location === "remote" ? "Remote" : "Local"}
                          {selections.location === "remote" && remoteHostInput.trim()
                            ? ` (${remoteHostInput.trim()})`
                            : ""}
                        </div>
                      </div>
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Source</div>
                        <div className="wizard-summary-v">
                          {selections.source === "clone"
                            ? `Clone repo${repoBranch.trim() ? ` (${repoBranch.trim()})` : ""}`
                            : selections.source === "import"
                              ? "Import folder"
                              : "New empty"}
                        </div>
                      </div>
                      {selections.source === "clone" && repoUrl.trim() && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Repo</div>
                          <div className="wizard-summary-v">{repoUrl.trim()}</div>
                        </div>
                      )}
                      {selections.source === "clone" && (sourcePath.trim() || useDiskIsolatedStaging) && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Destination</div>
                          <div className="wizard-summary-v">
                            {useDiskIsolatedStaging ? "Managed staging (container)" : sourcePath.trim()}
                          </div>
                        </div>
                      )}
                      {selections.source === "import" && sourcePath.trim() && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Folder</div>
                          <div className="wizard-summary-v">{sourcePath.trim()}</div>
                        </div>
                      )}
                      {selections.source === "new" && (sourcePath.trim() || useDiskIsolatedStaging) && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Destination</div>
                          <div className="wizard-summary-v">
                            {useDiskIsolatedStaging ? "Managed staging (container)" : sourcePath.trim()}
                          </div>
                        </div>
                      )}
                      {(selections.source === "new" || selections.source === "import") && workspaceName.trim() && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Name</div>
                          <div className="wizard-summary-v">{workspaceName.trim()}</div>
                        </div>
                      )}
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Sandbox</div>
                        <div className="wizard-summary-v">
                          {selections.container === "no-container"
                            ? "Host (no container)"
                            : selections.container === "host-mounted"
                              ? "Container (host-mounted)"
                              : "Container (disk-isolated)"}
                        </div>
                      </div>
                      {selections.container !== "no-container" && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Network</div>
                          <div className="wizard-summary-v">
                            {selections.network === "allowlist"
                              ? "Allowlist"
                              : selections.network === "full"
                                ? "Full access"
                                : "LLM providers only"}
                          </div>
                        </div>
                      )}
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Harness downloads</div>
                        <div className="wizard-summary-v">{harnessSummaryValue}</div>
                      </div>
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Task titling</div>
                        <div className="wizard-summary-v">{titlingSummaryValue}</div>
                      </div>
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Worktree hook</div>
                        <div className="wizard-summary-v">{setupHook.trim() || "(none)"}</div>
                      </div>
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Merge queue</div>
                        <div className="wizard-summary-v">
                          {mergeQueueSkipped
                            ? "Disabled"
                            : `Target ${targetBranch.trim() || "main"}${verifyCommand.trim() ? `, verify: ${verifyCommand.trim()}` : ""}`}
                        </div>
                      </div>
                      {!mergeQueueSkipped && pushOnSuccess && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Merge push</div>
                          <div className="wizard-summary-v">
                            {`${(pushRemote.trim() || "origin")}:${pushBranch.trim() || targetBranch.trim() || "main"}`}
                          </div>
                        </div>
                      )}
                    </div>
                  </div>
                )}
                {step.key === "setup" && (
                  <button
                    type="button"
                    className="wizard-skip wizard-skip--left wizard-skip--below"
                    onClick={onNext}
                  >
                    Skip for now
                  </button>
                )}
              </div>
            </div>
          </div>
          <div className="wizard-pagination" role="tablist" aria-label="Setup steps">
            {(() => {
              const isStepSatisfied = (key: string): boolean => {
                if (key === "location") {
                  if (selections.location === "local") return true;
                  if (selections.location !== "remote") return false;
                  return remoteStatus === "connected" && hasRemoteHost;
                }
                if (key === "auth-import") return true;
                if (key === "harness-downloads") return true;
                if (key === "session-titling") {
                  return titlingMode === "skip"
                    || titlingMode === "local"
                    || (titlingMode === "remote" && titlingRemoteValid);
                }
                if (key === "source") return sourceStepComplete;
                if (key === "merge-queue") return mergeQueueSkipped || Boolean(targetBranch.trim());
                return true;
              };

              let maxIdx = 0;
              for (let index = 0; index < steps.length; index += 1) {
                if (isStepSatisfied(steps[index].key)) {
                  maxIdx = Math.min(steps.length - 1, index + 1);
                } else {
                  maxIdx = Math.max(0, index);
                  break;
                }
              }

              return steps.map((item, index) => {
                const disabled = index > maxIdx;
                return (
                  <button
                    key={item.key}
                    type="button"
                    className={`wizard-dot${index === stepIndex ? " is-active" : ""}`}
                    aria-label={`Go to step ${index + 1}`}
                    aria-current={index === stepIndex ? "true" : undefined}
                    disabled={disabled}
                    onClick={() => {
                      if (!disabled) {
                        goToStepKey(item.key);
                      }
                    }}
                  />
                );
              });
            })()}
          </div>
          <div className="wizard-actions">
            {isFirst ? (
              <Link to="/" className="wizard-secondary" data-testid="wizard-back-link">
                Back
              </Link>
            ) : (
              <button
                type="button"
                className="wizard-secondary"
                data-testid="wizard-back"
                onClick={() => goRelativeStep(-1)}
              >
                Back
              </button>
            )}
            {isLast ? (
              <button
                type="button"
                className="wizard-primary"
                data-testid="wizard-create"
                disabled={!canAdvance || creating}
                onClick={onCreate}
              >
                {createButtonLabel}
              </button>
            ) : (
              <button
                type="button"
                className="wizard-primary"
                data-testid="wizard-next"
                disabled={!canAdvance || creating}
                onClick={onNext}
              >
                {nextButtonLabel}
              </button>
            )}
          </div>
        </div>
      </LauncherBrand>
    </div>
  );
}
