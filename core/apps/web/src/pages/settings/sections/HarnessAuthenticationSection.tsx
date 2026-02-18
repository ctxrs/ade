import type { CSSProperties } from "react";
import { Ellipsis, KeyRound, User as UserIcon, X } from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "../../../components/ui/dropdown-menu";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "../../../components/ui/select";
import { HARNESS_CATALOG, type HarnessCatalogEntry } from "../../../utils/harnessCatalog";
import { PROVIDER_INSTALLS_ENABLED } from "../../../utils/providerInstallGate";
import { Card, Row } from "../../SettingsPage.components";
import { clampPct } from "../../SettingsPage.utils";
import { buildHarnessAuthRows } from "../harnessAuthRows";
import {
  getHarnessEndpointProviderPreset,
  HARNESS_ENDPOINT_PROVIDER_PRESETS,
  supportsOptionalBaseUrlForHarness,
} from "../harnessEndpointProviders";
import { useHarnessAuthenticationController } from "../hooks/useHarnessAuthenticationController";

type HarnessAuthenticationSectionProps = {
  workspaceId: string | null;
  active: boolean;
};

export function HarnessAuthenticationSection({ workspaceId, active }: HarnessAuthenticationSectionProps) {
  const {
    providers,
    installs,
    installBusy,
    onInstallAll,
    onInstall,
    providerHarnessConfig,
    providerHarnessBusy,
    codexAccounts,
    codexAccountsBusy,
    claudeAccounts,
    claudeAccountsBusy,
    geminiAccounts,
    geminiAccountsBusy,
    kimiAccounts,
    kimiAccountsBusy,
    copilotAccounts,
    copilotAccountsBusy,
    kiroAccounts,
    kiroAccountsBusy,
    harnessAuthModal,
    openHarnessAuthModal,
    closeHarnessAuthModal,
    patchHarnessAuthModal,
    submitHarnessSubscriptionModal,
    submitHarnessApiKeyModal,
    onSelectHarnessAuthRow,
    onDeleteProviderEndpoint,
    onCodexDelete,
    onClaudeDelete,
    onGeminiDelete,
    onKimiDelete,
    onCopilotDelete,
    onKiroDelete,
    providerError,
    supportsHarnessEndpointConfig,
    harnessEndpointRequiresBaseUrl,
  } = useHarnessAuthenticationController({
    workspaceId,
    enabled: active,
  });

  const visibleProviders = providers.filter((provider) => provider.details?.ui_hidden !== "true").slice();
  const installControlsEnabled = PROVIDER_INSTALLS_ENABLED;
  const scopedVisibleProviders = installControlsEnabled
    ? visibleProviders
    : visibleProviders.filter((provider) => provider.installed === true && provider.health === "ok");
  const providersById = new Map(scopedVisibleProviders.map((provider) => [provider.provider_id, provider]));

  const order = new Map<string, number>(HARNESS_CATALOG.map((harness, index) => [harness.id, index]));
  const curated = HARNESS_CATALOG.filter((harness) => providersById.has(harness.id));
  const extras: HarnessCatalogEntry[] = scopedVisibleProviders
    .filter((provider) => !order.has(provider.provider_id))
    .map((provider) => ({ id: provider.provider_id, label: provider.provider_id, logoSrc: "" }))
    .sort((a, b) => a.id.localeCompare(b.id));

  const harnesses = [...curated, ...extras];
  const harnessDisplayById = new Map<
    string,
    { label: string; logoSrc: string; invertInDark?: boolean; invertInLight?: boolean }
  >(harnesses.map((entry) => [entry.id, entry]));
  const activeModalHarness = harnessAuthModal ? harnessDisplayById.get(harnessAuthModal.provider_id) : undefined;
  const activeModalEndpointPreset = harnessAuthModal
    ? getHarnessEndpointProviderPreset(harnessAuthModal.endpoint_provider_id)
    : null;
  const modalRequiresBaseUrl = harnessAuthModal
    ? harnessEndpointRequiresBaseUrl(harnessAuthModal.provider_id)
    : true;
  const modalAllowsCustomBaseUrl = activeModalEndpointPreset?.id === "other";
  const modalAllowsOptionalBaseUrl = harnessAuthModal
    ? supportsOptionalBaseUrlForHarness(harnessAuthModal.provider_id)
    : false;
  const showBaseUrlInput =
    (modalRequiresBaseUrl && modalAllowsCustomBaseUrl)
    || (!modalRequiresBaseUrl && modalAllowsOptionalBaseUrl);
  const modalApiKeyLabel = harnessAuthModal?.provider_id === "kiro" ? "Auth token JSON" : "API key";
  const modalApiKeyPlaceholder = harnessAuthModal?.provider_id === "kiro"
    ? '{"token":"..."}'
    : harnessAuthModal?.provider_id === "rovo"
      ? "Atlassian API token"
      : harnessAuthModal?.provider_id === "auggie"
        ? "Auggie session token"
        : "sk-...";

  return (
    <>
      <p className="settings-harness-intro">
        Authenticate each agent harness with the provider&apos;s subscription or API key.
      </p>
      {installControlsEnabled ? (
        <Card>
          <Row
            title="Install all"
            description="Installs supported harnesses to ~/.ctx/providers/agent-servers."
            control={
              <button
                type="button"
                className="settings-btn"
                onClick={() => {
                  void onInstallAll();
                }}
                disabled={installBusy !== null}
              >
                {installBusy === "all" ? "Installing…" : "Install all"}
              </button>
            }
          />
        </Card>
      ) : null}

      <div className="settings-card settings-harness-list">
        <div className="settings-card-rows">
          {harnesses.map((harness) => {
            const id = harness.id;
            const provider = providersById.get(id);
            if (!provider) return null;

            const installed = provider.installed === true && provider.health === "ok";
            const installSupported = provider.details?.install_supported === "true";
            const installUi = installs[id];
            const installRunning = installUi?.state === "running" || provider.details?.install_running === "true";
            const installBusyLocal = installBusy !== null || installRunning;
            const installLabel =
              installBusyLocal && installUi?.pct !== null
                ? `${clampPct(installUi.pct)}%`
                : installBusyLocal
                  ? "Installing…"
                  : provider.installed
                    ? "Update"
                    : "Install";

            const harnessCfg = providerHarnessConfig[id];
            const sourceBusy = providerHarnessBusy[id] || false;
            const authRows = buildHarnessAuthRows({
              provider_id: id,
              selected_source_kind: harnessCfg?.selected_source_kind ?? "subscription",
              selected_endpoint_id: harnessCfg?.selected_endpoint_id ?? null,
              endpoints: harnessCfg?.endpoints ?? [],
              codex_accounts: id === "codex" ? (codexAccounts?.accounts ?? []) : [],
              codex_active_account_id: id === "codex" ? (codexAccounts?.active_account_id ?? null) : null,
              claude_accounts: id === "claude-crp" ? (claudeAccounts?.accounts ?? []) : [],
              claude_active_account_id: id === "claude-crp" ? (claudeAccounts?.active_account_id ?? null) : null,
              gemini_accounts: id === "gemini" ? (geminiAccounts?.accounts ?? []) : [],
              gemini_active_account_id: id === "gemini" ? (geminiAccounts?.active_account_id ?? null) : null,
              kimi_accounts: id === "kimi" ? (kimiAccounts?.accounts ?? []) : [],
              kimi_active_account_id: id === "kimi" ? (kimiAccounts?.active_account_id ?? null) : null,
              copilot_accounts: id === "copilot" ? (copilotAccounts?.accounts ?? []) : [],
              copilot_active_account_id: id === "copilot" ? (copilotAccounts?.active_account_id ?? null) : null,
              kiro_accounts: id === "kiro" ? (kiroAccounts?.accounts ?? []) : [],
              kiro_active_account_id: id === "kiro" ? (kiroAccounts?.active_account_id ?? null) : null,
            });
            const addBusy = harnessAuthModal?.provider_id === id
              ? harnessAuthModal.api_key_busy || harnessAuthModal.subscription_busy
              : false;
            const rowBusy =
              sourceBusy
              || (id === "codex" && codexAccountsBusy)
              || (id === "claude-crp" && claudeAccountsBusy)
              || (id === "gemini" && geminiAccountsBusy)
              || (id === "kimi" && kimiAccountsBusy)
              || (id === "copilot" && copilotAccountsBusy)
              || (id === "kiro" && kiroAccountsBusy);

            const installStyle: CSSProperties | undefined =
              installBusyLocal && installUi?.pct !== null
                ? ({ ["--settings-install-pct" as "--settings-install-pct"]: `${clampPct(installUi.pct)}%` } as CSSProperties)
                : undefined;

            return (
              <div key={id} className={`settings-row settings-harness-row ${installed ? "" : "settings-harness-row-disabled"}`}>
                <div className="settings-row-left">
                  <div className="settings-row-title settings-harness-title">
                    {harness.logoSrc ? (
                      <img
                        className={`settings-harness-logo ${harness.invertInDark ? "wb-invert" : ""} ${
                          harness.invertInLight ? "wb-invert-light" : ""
                        }`}
                        src={harness.logoSrc}
                        alt=""
                      />
                    ) : (
                      <span className="settings-harness-logo-fallback" aria-hidden="true" />
                    )}
                    <span className="settings-harness-name">{harness.label}</span>
                    {installed ? (
                      <button
                        type="button"
                        className="settings-harness-add"
                        onClick={() => openHarnessAuthModal(id)}
                        disabled={addBusy}
                        title="Add authentication method"
                        aria-label={`Add auth for ${harness.label}`}
                      >
                        +
                      </button>
                    ) : null}
                  </div>
                  {installed && authRows.length > 0 ? (
                    <div className="settings-harness-auth-list" style={{ marginTop: 10 }}>
                      {authRows.map((row) => {
                        const verificationLabel =
                          row.verification_status && row.verification_status !== "unknown"
                            ? row.verification_status
                            : null;
                        const verificationClass =
                          verificationLabel === "valid"
                            ? "settings-pill-ok"
                            : verificationLabel === "invalid" || verificationLabel === "error"
                              ? "settings-pill-err"
                              : "";

                        return (
                          <div
                            key={row.key}
                            className={`settings-harness-auth-row ${row.active ? "settings-harness-auth-row-active" : ""}`}
                          >
                            <button
                              type="button"
                              className="settings-harness-auth-main"
                              onClick={() => {
                                void onSelectHarnessAuthRow(id, row);
                              }}
                              disabled={rowBusy || !row.selectable}
                            >
                              <span className="settings-harness-auth-kind">
                                {row.kind === "subscription" ? "Subscription" : "API Key"}
                              </span>
                              <span className="settings-harness-auth-primary">
                                <span className="settings-harness-auth-label">{row.label}</span>
                                {row.detail ? <span className="settings-harness-auth-detail">{row.detail}</span> : null}
                              </span>
                            </button>
                            <div className="settings-harness-auth-actions">
                              {verificationLabel ? (
                                <span className={`settings-pill ${verificationClass}`}>{verificationLabel}</span>
                              ) : null}
                              {row.last_error ? (
                                <span className="settings-pill settings-pill-err" title={row.last_error}>
                                  Error
                                </span>
                              ) : null}
                              {row.active ? <span className="settings-pill settings-pill-ok">Active</span> : null}
                              <DropdownMenu>
                                <DropdownMenuTrigger asChild>
                                  <button
                                    type="button"
                                    className="settings-harness-auth-menu-trigger"
                                    disabled={rowBusy}
                                    aria-label="More actions"
                                    title="More actions"
                                  >
                                    <Ellipsis size={14} aria-hidden="true" />
                                  </button>
                                </DropdownMenuTrigger>
                                <DropdownMenuContent align="end">
                                  {row.active ? <DropdownMenuItem disabled>Active source</DropdownMenuItem> : null}
                                  {row.selectable && !row.active ? (
                                    <DropdownMenuItem
                                      onSelect={() => {
                                        void onSelectHarnessAuthRow(id, row);
                                      }}
                                    >
                                      Set active
                                    </DropdownMenuItem>
                                  ) : null}
                                  {row.endpoint_id && row.can_delete ? (
                                    <DropdownMenuItem
                                      className="tw-text-[var(--error-contrast)] focus:tw-bg-[var(--error-soft)]"
                                      onSelect={() => {
                                        void onDeleteProviderEndpoint(id, row.endpoint_id!);
                                      }}
                                    >
                                      Delete
                                    </DropdownMenuItem>
                                  ) : null}
                                  {row.kind === "subscription" && id === "codex" && row.account_id && row.can_delete ? (
                                    <DropdownMenuItem
                                      className="tw-text-[var(--error-contrast)] focus:tw-bg-[var(--error-soft)]"
                                      onSelect={() => {
                                        const accountId = row.account_id;
                                        if (!accountId) return;
                                        void onCodexDelete(accountId);
                                      }}
                                    >
                                      Delete
                                    </DropdownMenuItem>
                                  ) : null}
                                  {row.kind === "subscription" && id === "claude-crp" && row.account_id && row.can_delete ? (
                                    <DropdownMenuItem
                                      className="tw-text-[var(--error-contrast)] focus:tw-bg-[var(--error-soft)]"
                                      onSelect={() => {
                                        const accountId = row.account_id;
                                        if (!accountId) return;
                                        void onClaudeDelete(accountId);
                                      }}
                                    >
                                      Delete
                                    </DropdownMenuItem>
                                  ) : null}
                                  {row.kind === "subscription" && id === "gemini" && row.account_id && row.can_delete ? (
                                    <DropdownMenuItem
                                      className="tw-text-[var(--error-contrast)] focus:tw-bg-[var(--error-soft)]"
                                      onSelect={() => {
                                        const accountId = row.account_id;
                                        if (!accountId) return;
                                        void onGeminiDelete(accountId);
                                      }}
                                    >
                                      Delete
                                    </DropdownMenuItem>
                                  ) : null}
                                  {row.kind === "subscription" && id === "kimi" && row.account_id && row.can_delete ? (
                                    <DropdownMenuItem
                                      className="tw-text-[var(--error-contrast)] focus:tw-bg-[var(--error-soft)]"
                                      onSelect={() => {
                                        const accountId = row.account_id;
                                        if (!accountId) return;
                                        void onKimiDelete(accountId);
                                      }}
                                    >
                                      Delete
                                    </DropdownMenuItem>
                                  ) : null}
                                  {row.kind === "subscription" && id === "copilot" && row.account_id && row.can_delete ? (
                                    <DropdownMenuItem
                                      className="tw-text-[var(--error-contrast)] focus:tw-bg-[var(--error-soft)]"
                                      onSelect={() => {
                                        const accountId = row.account_id;
                                        if (!accountId) return;
                                        void onCopilotDelete(accountId);
                                      }}
                                    >
                                      Delete
                                    </DropdownMenuItem>
                                  ) : null}
                                  {row.kind === "subscription" && id === "kiro" && row.account_id && row.can_delete ? (
                                    <DropdownMenuItem
                                      className="tw-text-[var(--error-contrast)] focus:tw-bg-[var(--error-soft)]"
                                      onSelect={() => {
                                        const accountId = row.account_id;
                                        if (!accountId) return;
                                        void onKiroDelete(accountId);
                                      }}
                                    >
                                      Delete
                                    </DropdownMenuItem>
                                  ) : null}
                                </DropdownMenuContent>
                              </DropdownMenu>
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  ) : null}
                </div>
                {!installed ? (
                  <div className="settings-row-right settings-harness-actions">
                    {installControlsEnabled ? (
                      <button
                        type="button"
                        className="settings-btn settings-btn-secondary"
                        onClick={() => {
                          void onInstall(id);
                        }}
                        disabled={!installSupported || installBusyLocal}
                        style={installStyle}
                        title={!installSupported ? "Install not supported yet" : "Install this harness"}
                      >
                        {installLabel}
                      </button>
                    ) : null}
                  </div>
                ) : null}
              </div>
            );
          })}
          {harnesses.length === 0 ? <div className="settings-empty">No harnesses.</div> : null}
        </div>
      </div>

      {harnessAuthModal ? (
        <div className="modal-overlay" role="dialog" aria-modal="true" onClick={closeHarnessAuthModal}>
          <div className="modal settings-harness-modal" onClick={(e) => e.stopPropagation()}>
            <div className="settings-harness-modal-header">
              <div className="settings-harness-title">
                {activeModalHarness?.logoSrc ? (
                  <img
                    className={`settings-harness-logo ${activeModalHarness.invertInDark ? "wb-invert" : ""} ${
                      activeModalHarness.invertInLight ? "wb-invert-light" : ""
                    }`}
                    src={activeModalHarness.logoSrc}
                    alt=""
                  />
                ) : (
                  <span className="settings-harness-logo-fallback" aria-hidden="true" />
                )}
                <span className="settings-harness-name">
                  {activeModalHarness?.label ?? harnessAuthModal.provider_id}
                </span>
              </div>
              <button
                type="button"
                className="settings-harness-modal-close"
                onClick={closeHarnessAuthModal}
                aria-label="Close"
              >
                <X size={16} aria-hidden="true" />
              </button>
            </div>

            {harnessAuthModal.stage === "choose" ? (
              <div className="settings-harness-modal-choice-stack">
                <div className="settings-harness-modal-choice-grid">
                  <button
                    type="button"
                    className="settings-btn settings-harness-modal-choice-btn"
                    onClick={() => {
                      patchHarnessAuthModal({
                        stage: "subscription",
                        subscription_status: null,
                        subscription_label: "",
                        subscription_token: "",
                        subscription_email: "",
                        subscription_provider: "",
                        subscription_credentials_json: "",
                        subscription_config_toml: "",
                        subscription_auth_token_json: "",
                        subscription_oauth_creds_json: "",
                        subscription_google_accounts_json: "",
                      });
                      if (harnessAuthModal.provider_id === "codex") {
                        void submitHarnessSubscriptionModal();
                      }
                    }}
                    disabled={harnessAuthModal.subscription_busy || harnessAuthModal.api_key_busy}
                  >
                    <UserIcon size={18} className="settings-harness-modal-choice-icon" aria-hidden="true" />
                    <span>Subscription</span>
                  </button>
                  <button
                    type="button"
                    className="settings-btn settings-harness-modal-choice-btn"
                    onClick={() =>
                      patchHarnessAuthModal({
                        stage: "api_key",
                        subscription_status: null,
                        subscription_label: "",
                        subscription_token: "",
                        subscription_email: "",
                        subscription_provider: "",
                        subscription_credentials_json: "",
                        subscription_config_toml: "",
                        subscription_auth_token_json: "",
                        subscription_oauth_creds_json: "",
                        subscription_google_accounts_json: "",
                        base_url: modalRequiresBaseUrl
                          ? (getHarnessEndpointProviderPreset(harnessAuthModal.endpoint_provider_id).base_url
                            ?? harnessAuthModal.base_url)
                          : "",
                      })}
                    disabled={!supportsHarnessEndpointConfig(harnessAuthModal.provider_id)}
                    title={
                      supportsHarnessEndpointConfig(harnessAuthModal.provider_id)
                        ? "Add API key auth"
                        : "API key auth not supported for this harness yet"
                    }
                  >
                    <KeyRound size={18} className="settings-harness-modal-choice-icon" aria-hidden="true" />
                    <span>API Key</span>
                  </button>
                </div>
              </div>
            ) : harnessAuthModal.stage === "subscription" ? (
              <div className="settings-harness-modal-fields">
                <div className="settings-row-desc">
                  {harnessAuthModal.provider_id === "codex"
                    ? "Sign in with your Codex subscription in a browser window."
                    : harnessAuthModal.provider_id === "claude-crp"
                      ? "Use a Claude subscription auth token (from `claude setup-token`) and set it as the managed account."
                      : harnessAuthModal.provider_id === "gemini"
                        ? "Paste Gemini OAuth credentials JSON (from oauth_creds.json) for a managed subscription account."
                        : harnessAuthModal.provider_id === "kimi"
                          ? "Paste Kimi credentials JSON for a managed Kimi share directory."
                          : harnessAuthModal.provider_id === "copilot"
                            ? "Paste a GitHub token with Copilot entitlement for the managed Copilot account."
                            : harnessAuthModal.provider_id === "kiro"
                              ? "Paste the Kiro auth token JSON for a managed token cache."
                      : "Authenticate this harness for the selected workspace."}
                </div>
                {harnessAuthModal.provider_id === "claude-crp" ? (
                  <>
                    <label className="settings-harness-modal-label">
                      Label (optional)
                      <input
                        className="settings-control"
                        value={harnessAuthModal.subscription_label}
                        onChange={(e) => patchHarnessAuthModal({ subscription_label: e.target.value })}
                        placeholder="Claude subscription"
                        autoFocus
                      />
                    </label>
                    <label className="settings-harness-modal-label">
                      Auth Token
                      <input
                        className="settings-control"
                        value={harnessAuthModal.subscription_token}
                        onChange={(e) => patchHarnessAuthModal({ subscription_token: e.target.value })}
                        placeholder="Paste ANTHROPIC_AUTH_TOKEN"
                        type="password"
                      />
                    </label>
                  </>
                ) : null}
                {harnessAuthModal.provider_id === "gemini" ? (
                  <>
                    <label className="settings-harness-modal-label">
                      Label (optional)
                      <input
                        className="settings-control"
                        value={harnessAuthModal.subscription_label}
                        onChange={(e) => patchHarnessAuthModal({ subscription_label: e.target.value })}
                        placeholder="Gemini subscription"
                        autoFocus
                      />
                    </label>
                    <label className="settings-harness-modal-label">
                      Email (optional)
                      <input
                        className="settings-control"
                        value={harnessAuthModal.subscription_email}
                        onChange={(e) => patchHarnessAuthModal({ subscription_email: e.target.value })}
                        placeholder="you@example.com"
                      />
                    </label>
                    <label className="settings-harness-modal-label">
                      OAuth Credentials JSON
                      <textarea
                        className="settings-control settings-control-wide"
                        value={harnessAuthModal.subscription_oauth_creds_json}
                        onChange={(e) => patchHarnessAuthModal({ subscription_oauth_creds_json: e.target.value })}
                        placeholder='{"access_token":"...","refresh_token":"..."}'
                        rows={6}
                      />
                    </label>
                    <label className="settings-harness-modal-label">
                      Google Accounts JSON (optional)
                      <textarea
                        className="settings-control settings-control-wide"
                        value={harnessAuthModal.subscription_google_accounts_json}
                        onChange={(e) => patchHarnessAuthModal({ subscription_google_accounts_json: e.target.value })}
                        placeholder='[{"email":"you@example.com"}]'
                        rows={4}
                      />
                    </label>
                  </>
                ) : null}
                {harnessAuthModal.provider_id === "kimi" ? (
                  <>
                    <label className="settings-harness-modal-label">
                      Label (optional)
                      <input
                        className="settings-control"
                        value={harnessAuthModal.subscription_label}
                        onChange={(e) => patchHarnessAuthModal({ subscription_label: e.target.value })}
                        placeholder="Kimi subscription"
                        autoFocus
                      />
                    </label>
                    <label className="settings-harness-modal-label">
                      Email (optional)
                      <input
                        className="settings-control"
                        value={harnessAuthModal.subscription_email}
                        onChange={(e) => patchHarnessAuthModal({ subscription_email: e.target.value })}
                        placeholder="you@example.com"
                      />
                    </label>
                    <label className="settings-harness-modal-label">
                      Provider (optional)
                      <input
                        className="settings-control"
                        value={harnessAuthModal.subscription_provider}
                        onChange={(e) => patchHarnessAuthModal({ subscription_provider: e.target.value })}
                        placeholder="moonshot"
                      />
                    </label>
                    <label className="settings-harness-modal-label">
                      Credentials JSON
                      <textarea
                        className="settings-control settings-control-wide"
                        value={harnessAuthModal.subscription_credentials_json}
                        onChange={(e) => patchHarnessAuthModal({ subscription_credentials_json: e.target.value })}
                        placeholder='{"access_token":"...","refresh_token":"..."}'
                        rows={6}
                      />
                    </label>
                    <label className="settings-harness-modal-label">
                      Config TOML (optional)
                      <textarea
                        className="settings-control settings-control-wide"
                        value={harnessAuthModal.subscription_config_toml}
                        onChange={(e) => patchHarnessAuthModal({ subscription_config_toml: e.target.value })}
                        placeholder='current_provider = "moonshot"'
                        rows={4}
                      />
                    </label>
                  </>
                ) : null}
                {harnessAuthModal.provider_id === "copilot" ? (
                  <>
                    <label className="settings-harness-modal-label">
                      Label (optional)
                      <input
                        className="settings-control"
                        value={harnessAuthModal.subscription_label}
                        onChange={(e) => patchHarnessAuthModal({ subscription_label: e.target.value })}
                        placeholder="Copilot subscription"
                        autoFocus
                      />
                    </label>
                    <label className="settings-harness-modal-label">
                      Email (optional)
                      <input
                        className="settings-control"
                        value={harnessAuthModal.subscription_email}
                        onChange={(e) => patchHarnessAuthModal({ subscription_email: e.target.value })}
                        placeholder="you@example.com"
                      />
                    </label>
                    <label className="settings-harness-modal-label">
                      Token
                      <input
                        className="settings-control"
                        value={harnessAuthModal.subscription_token}
                        onChange={(e) => patchHarnessAuthModal({ subscription_token: e.target.value })}
                        placeholder="ghp_..."
                        type="password"
                      />
                    </label>
                  </>
                ) : null}
                {harnessAuthModal.provider_id === "kiro" ? (
                  <>
                    <label className="settings-harness-modal-label">
                      Label (optional)
                      <input
                        className="settings-control"
                        value={harnessAuthModal.subscription_label}
                        onChange={(e) => patchHarnessAuthModal({ subscription_label: e.target.value })}
                        placeholder="Kiro subscription"
                        autoFocus
                      />
                    </label>
                    <label className="settings-harness-modal-label">
                      Email (optional)
                      <input
                        className="settings-control"
                        value={harnessAuthModal.subscription_email}
                        onChange={(e) => patchHarnessAuthModal({ subscription_email: e.target.value })}
                        placeholder="you@example.com"
                      />
                    </label>
                    <label className="settings-harness-modal-label">
                      Auth Token JSON
                      <textarea
                        className="settings-control settings-control-wide"
                        value={harnessAuthModal.subscription_auth_token_json}
                        onChange={(e) => patchHarnessAuthModal({ subscription_auth_token_json: e.target.value })}
                        placeholder='{"accessToken":"...","expiresAt":"..."}'
                        rows={6}
                      />
                    </label>
                  </>
                ) : null}
                {harnessAuthModal.subscription_status ? (
                  <div className="settings-row-desc settings-harness-modal-status">
                    {harnessAuthModal.subscription_status}
                  </div>
                ) : null}
                <div className="modal-actions settings-harness-modal-actions">
                  <button
                    type="button"
                    className="settings-btn settings-btn-secondary"
                    onClick={() =>
                      patchHarnessAuthModal({
                        stage: "choose",
                        subscription_status: null,
                        subscription_token: "",
                        subscription_email: "",
                        subscription_provider: "",
                        subscription_credentials_json: "",
                        subscription_config_toml: "",
                        subscription_auth_token_json: "",
                        subscription_oauth_creds_json: "",
                        subscription_google_accounts_json: "",
                      })}
                    disabled={harnessAuthModal.subscription_busy}
                  >
                    Back
                  </button>
                  <button
                    type="button"
                    className="settings-btn"
                    onClick={() => {
                      void submitHarnessSubscriptionModal();
                    }}
                    disabled={harnessAuthModal.subscription_busy || harnessAuthModal.api_key_busy}
                  >
                    {harnessAuthModal.subscription_busy
                      ? "Starting..."
                      : harnessAuthModal.provider_id === "codex"
                        ? "Start sign-in"
                        : "Save subscription"}
                  </button>
                </div>
              </div>
            ) : (
              <div className="settings-harness-modal-fields">
                {modalRequiresBaseUrl ? (
                  <label className="settings-harness-modal-label">
                    Provider
                    <Select
                      value={harnessAuthModal.endpoint_provider_id}
                      onValueChange={(nextProviderId) => {
                        const preset = getHarnessEndpointProviderPreset(nextProviderId);
                        patchHarnessAuthModal({
                          endpoint_provider_id: nextProviderId,
                          base_url: preset.base_url ?? "",
                        });
                      }}
                    >
                      <SelectTrigger className="tw-min-w-[10rem]">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {HARNESS_ENDPOINT_PROVIDER_PRESETS.map((preset) => (
                          <SelectItem key={preset.id} value={preset.id}>
                            {preset.label}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </label>
                ) : null}
                <label className="settings-harness-modal-label">
                  {modalApiKeyLabel}
                  <input
                    className="settings-control settings-control-wide"
                    type="password"
                    placeholder={modalApiKeyPlaceholder}
                    value={harnessAuthModal.api_key}
                    onChange={(e) => patchHarnessAuthModal({ api_key: e.target.value })}
                  />
                </label>
                <label className="settings-harness-modal-label">
                  Name (optional)
                  <input
                    className="settings-control settings-control-wide"
                    value={harnessAuthModal.endpoint_name}
                    onChange={(e) => patchHarnessAuthModal({ endpoint_name: e.target.value })}
                  />
                </label>
                {showBaseUrlInput ? (
                  <label className="settings-harness-modal-label">
                    Base URL{modalRequiresBaseUrl ? "" : " (optional)"}
                    <input
                      className="settings-control settings-control-wide"
                      placeholder="https://api.example.com/v1"
                      value={harnessAuthModal.base_url}
                      onChange={(e) => patchHarnessAuthModal({ base_url: e.target.value })}
                    />
                  </label>
                ) : null}
                <div className="modal-actions settings-harness-modal-actions">
                  <button
                    type="button"
                    className="settings-btn settings-btn-secondary"
                    onClick={() =>
                      patchHarnessAuthModal({
                        stage: "choose",
                        api_key: "",
                        subscription_status: null,
                      })}
                  >
                    Back
                  </button>
                  <button
                    type="button"
                    className="settings-btn"
                    onClick={() => {
                      void submitHarnessApiKeyModal();
                    }}
                    disabled={harnessAuthModal.api_key_busy || harnessAuthModal.subscription_busy}
                  >
                    {harnessAuthModal.api_key_busy ? "Saving..." : "Add API key"}
                  </button>
                </div>
              </div>
            )}
          </div>
        </div>
      ) : null}

      {providerError ? <div className="settings-banner settings-banner-error">{providerError}</div> : null}
    </>
  );
}
