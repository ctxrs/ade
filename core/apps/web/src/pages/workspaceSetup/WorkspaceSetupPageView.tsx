import { Link } from "react-router-dom";
import { ChevronRight } from "lucide-react";
import LauncherBrand from "../../components/LauncherBrand";
import {
  AuthImportStepPanel,
  HarnessDownloadsStepPanel,
} from "./WorkspaceSetupPanels";
import {
  WorkspaceLaunchLogPanel,
  WorkspaceSetupDialogs,
  WorkspaceSetupStepOptions,
} from "./WorkspaceSetupChrome";
import { WorkspaceSetupPagination } from "./WorkspaceSetupPagination";
import { WorkspaceSetupStepHeader } from "./WorkspaceSetupStepHeader";
import type { WorkspaceSetupPageViewProps } from "./WorkspaceSetupPageView.types";

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
  networkAllowlist,
  setNetworkAllowlist,
  remoteHostInput,
  onRemoteInputChange,
  remotePortInput,
  onRemotePortInputChange,
  remoteDataDirInput,
  onRemoteDataDirInputChange,
  localAdminPasswordPromptVisible,
  localAdminPasswordInput,
  setLocalAdminPasswordInput,
  remotePasswordPromptVisible,
  remotePasswordPromptMode,
  remotePasswordInput,
  onRemotePasswordInputChange,
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
  useSandboxStaging,
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
          <WorkspaceSetupDialogs
            importInitDialog={importInitDialog}
            resolveImportInitDialog={resolveImportInitDialog}
            infoStep={infoStep}
            setOpenInfoKey={setOpenInfoKey}
          />
          <div className="wizard-steps">
            <div className="wizard-step" data-testid="wizard-step" data-step-key={step.key}>
              <WorkspaceSetupStepHeader step={step} setOpenInfoKey={setOpenInfoKey} />
              <div className="wizard-step-body">
                {createError && (
                  <div className="wizard-error">{createError}</div>
                )}
                <WorkspaceLaunchLogPanel
                  showLaunchPanel={showLaunchPanel && step.key === "confirm"}
                  launchSnapshot={launchSnapshot}
                  currentLaunchStepLabel={currentLaunchStepLabel}
                  currentLaunchElapsed={currentLaunchElapsed}
                  currentLaunchEtaLabel={currentLaunchEtaLabel}
                  launchCopyLabel={launchCopyLabel}
                  onCopyLaunchDiagnostics={onCopyLaunchDiagnostics}
                  launchLogs={launchLogs}
                />
                <WorkspaceSetupStepOptions
                  step={step}
                  selections={selections}
                  onSelectOption={onSelectOption}
                />
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
                    <div className="wizard-input">
                      <label>
                        Remote daemon port
                        <input
                          data-testid="wizard-remote-port"
                          inputMode="numeric"
                          placeholder="4399"
                          value={remotePortInput}
                          onChange={(event) => onRemotePortInputChange(event.target.value)}
                        />
                      </label>
                    </div>
                    <div className="wizard-input">
                      <label>
                        Remote data directory
                        <input
                          data-testid="wizard-remote-data-dir"
                          placeholder="Optional; defaults to ~/.ctx"
                          value={remoteDataDirInput}
                          onChange={(event) => onRemoteDataDirInputChange(event.target.value)}
                        />
                      </label>
                    </div>
                    {remotePasswordPromptVisible ? (
                      <div className="wizard-input">
                        <label>
                          {remotePasswordPromptMode === "admin" ? "Remote Admin Password" : "SSH Password"}
                          <input
                            data-testid="wizard-remote-password-once"
                            type="password"
                            autoComplete="current-password"
                              value={remotePasswordInput}
                              onChange={(event) => {
                                setCreateError(null);
                                onRemotePasswordInputChange(event.target.value);
                                if (remoteStatus !== "idle") {
                                  setRemoteStatus("idle");
                                  setRemoteError(null);
                              }
                            }}
                          />
                        </label>
                        <div className="wizard-note">
                          {remotePasswordPromptMode === "admin"
                            ? "Used once to finish sandbox setup on this host; never stored"
                            : "Used to install SSH key auth; never stored"}
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
                {step.key === "location" && selections.location === "local" && localAdminPasswordPromptVisible && (
                  <div className="wizard-remote">
                    <div className="wizard-input">
                      <label>
                        Linux Admin Password
                        <input
                          data-testid="wizard-local-admin-password-once"
                          type="password"
                          autoComplete="current-password"
                          value={localAdminPasswordInput}
                          onChange={(event) => {
                            setCreateError(null);
                            setLocalAdminPasswordInput(event.target.value);
                          }}
                        />
                      </label>
                      <div className="wizard-note">
                        Used once to finish sandbox setup on this machine; never stored
                      </div>
                    </div>
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
                    {selections.container !== "host" && (
                      <div className="wizard-note">
                        This is the project folder on the host. In sandbox mode, ctx will copy the workspace into an isolated managed filesystem for execution.
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
                    {useSandboxStaging && (
                      <div className="wizard-note">
                        Ctx will clone into a managed staging path under <code>~/.ctx/workspaces/staging/</code> by default. Your workspace will live in the sandbox.
                      </div>
                    )}
                    {!useSandboxStaging && (
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
                    {selections.source === "new" && useSandboxStaging && (
                      <div className="wizard-note">
                        Ctx will create the repo in a managed staging path. Your workspace will live in the sandbox.
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
                      {selections.source === "clone" && (sourcePath.trim() || useSandboxStaging) && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Destination</div>
                          <div className="wizard-summary-v">
                            {useSandboxStaging ? "Managed staging (sandbox)" : sourcePath.trim()}
                          </div>
                        </div>
                      )}
                      {selections.source === "import" && sourcePath.trim() && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Folder</div>
                          <div className="wizard-summary-v">{sourcePath.trim()}</div>
                        </div>
                      )}
                      {selections.source === "new" && (sourcePath.trim() || useSandboxStaging) && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Destination</div>
                          <div className="wizard-summary-v">
                            {useSandboxStaging ? "Managed staging (sandbox)" : sourcePath.trim()}
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
                          {selections.container === "host"
                            ? "Host"
                            : "Sandbox"}
                        </div>
                      </div>
                      {selections.container !== "host" && (
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
          <WorkspaceSetupPagination
            steps={steps}
            stepIndex={stepIndex}
            selections={selections}
            remoteStatus={remoteStatus}
            hasRemoteHost={hasRemoteHost}
            titlingMode={titlingMode}
            titlingRemoteValid={titlingRemoteValid}
            sourceStepComplete={sourceStepComplete}
            mergeQueueSkipped={mergeQueueSkipped}
            targetBranch={targetBranch}
            goToStepKey={goToStepKey}
          />
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
