import { useEffect, useRef, type CSSProperties } from "react";
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
import { HARNESS_CATALOG, type HarnessCatalogEntry, UNSUPPORTED_HARNESS_IDS } from "../../../utils/harnessCatalog";
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
import type { HarnessAuthModalState } from "../../SettingsPage.types";

type HarnessAuthenticationSectionProps = {
  workspaceId: string | null;
  active: boolean;
  modalOnly?: boolean;
  openProviderId?: string | null;
  onModalClosed?: (providerId: string | null) => void;
};

function claudeSetupTokenProvided(modal: HarnessAuthModalState): boolean {
  return modal.provider_id === "claude-crp" && modal.subscription_token.trim().length > 0;
}

function isClaudeSetupTokenValue(value: string): boolean {
  return value.trim().startsWith("sk-ant-oat");
}

export function canSubmitSubscriptionModal(modal: HarnessAuthModalState): boolean {
  if (modal.api_key_busy) return false;
  if (!modal.subscription_busy) return true;
  return claudeSetupTokenProvided(modal);
}

export function subscriptionPrimaryActionLabel(modal: HarnessAuthModalState): string {
  if (modal.provider_id === "claude-crp" && modal.subscription_busy && claudeSetupTokenProvided(modal)) {
    return isClaudeSetupTokenValue(modal.subscription_token) ? "Save subscription" : "Submit code";
  }
  if (modal.subscription_busy && !canSubmitSubscriptionModal(modal)) {
    return "Waiting...";
  }
  if (
    modal.provider_id === "codex"
    || modal.provider_id === "copilot"
    || modal.provider_id === "cursor"
    || modal.provider_id === "amp"
    || modal.provider_id === "auggie"
    || modal.provider_id === "gemini"
    || modal.provider_id === "kimi"
    || (modal.provider_id === "claude-crp" && !claudeSetupTokenProvided(modal))
  ) {
    if (modal.provider_id === "gemini") return "Sign in with Google";
    if (modal.provider_id === "kimi") return "Sign in with Kimi";
    if (modal.provider_id === "copilot") return "Sign in with GitHub";
    return "Start sign-in";
  }
  return "Save subscription";
}

export function shouldSubmitClaudeFallbackOnEnter(modal: HarnessAuthModalState, key: string): boolean {
  if (key !== "Enter") return false;
  if (modal.provider_id !== "claude-crp") return false;
  return claudeSetupTokenProvided(modal) && canSubmitSubscriptionModal(modal);
}

export function shouldAutoStartSubscriptionFlow(providerId: string): boolean {
  return providerId === "codex"
    || providerId === "claude-crp"
    || providerId === "gemini"
    || providerId === "kimi"
    || providerId === "amp"
    || providerId === "copilot"
    || providerId === "auggie";
}

export function HarnessAuthenticationSection({
  workspaceId,
  active,
  modalOnly = false,
  openProviderId,
  onModalClosed,
}: HarnessAuthenticationSectionProps) {
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
    cursorAccounts,
    cursorAccountsBusy,
    ampAccounts,
    ampAccountsBusy,
    auggieAccounts,
    auggieAccountsBusy,
    harnessAuthModal,
    openHarnessAuthModal,
    closeHarnessAuthModal,
    patchHarnessAuthModal,
    submitHarnessSubscriptionModal,
    submitHarnessApiKeyModal,
    onSelectHarnessAuthRow,
    onDeleteProviderEndpoint,
    onRefreshProviderEndpointModels,
    onCodexDelete,
    onClaudeDelete,
    onGeminiDelete,
    onKimiDelete,
    onCopilotDelete,
    onKiroDelete,
    onCursorDelete,
    onAmpDelete,
    onAuggieDelete,
    providerError,
    supportsHarnessEndpointConfig,
    harnessEndpointRequiresBaseUrl,
  } = useHarnessAuthenticationController({
    workspaceId,
    enabled: active,
  });

  const visibleProviders = providers
    .filter((provider) => provider.details?.ui_hidden !== "true")
    .filter((provider) => !UNSUPPORTED_HARNESS_IDS.has(provider.provider_id))
    .slice();
  const installControlsEnabled = PROVIDER_INSTALLS_ENABLED;
  const scopedVisibleProviders = visibleProviders;
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
  const modalSupportsApiKey =
    harnessAuthModal === null
      ? false
      : harnessAuthModal.provider_id === "cursor"
        || supportsHarnessEndpointConfig(harnessAuthModal.provider_id);
  const modalApiKeyLabel = harnessAuthModal?.provider_id === "kiro"
    ? "Auth token JSON"
    : harnessAuthModal?.provider_id === "gemini"
      ? harnessAuthModal.gemini_endpoint_auth_type === "vertex_ai"
        ? "Google API key"
        : "Gemini API key"
      : "API key";
  const modalApiKeyPlaceholder = harnessAuthModal?.provider_id === "kiro"
    ? '{"token":"..."}'
    : harnessAuthModal?.provider_id === "gemini"
      ? "AIza..."
    : harnessAuthModal?.provider_id === "auggie"
        ? "Auggie session token"
        : "sk-...";
  const modalSubscriptionAuthUrl = harnessAuthModal?.subscription_auth_url?.trim() ?? "";
  const lastModalProviderIdRef = useRef<string | null>(null);
  const suppressReopenProviderIdRef = useRef<string | null>(null);

  useEffect(() => {
    if (!active) return;
    if (!openProviderId) return;
    if (!harnessAuthModal && openProviderId === lastModalProviderIdRef.current) return;
    if (openProviderId === suppressReopenProviderIdRef.current) return;
    if (harnessAuthModal?.provider_id === openProviderId) return;
    openHarnessAuthModal(openProviderId);
  }, [active, harnessAuthModal?.provider_id, openHarnessAuthModal, openProviderId]);

  useEffect(() => {
    if (!openProviderId || openProviderId !== suppressReopenProviderIdRef.current) {
      suppressReopenProviderIdRef.current = null;
    }
  }, [openProviderId]);

  useEffect(() => {
    if (harnessAuthModal) {
      lastModalProviderIdRef.current = harnessAuthModal.provider_id;
      return;
    }
    if (!lastModalProviderIdRef.current) return;
    const closedProviderId = lastModalProviderIdRef.current;
    lastModalProviderIdRef.current = null;
    suppressReopenProviderIdRef.current = closedProviderId;
    onModalClosed?.(closedProviderId);
  }, [harnessAuthModal, onModalClosed]);

  return (
    <>
      {!modalOnly ? (
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
              cursor_accounts: id === "cursor" ? (cursorAccounts?.accounts ?? []) : [],
              cursor_active_account_id: id === "cursor" ? (cursorAccounts?.active_account_id ?? null) : null,
              amp_accounts: id === "amp" ? (ampAccounts?.accounts ?? []) : [],
              amp_active_account_id: id === "amp" ? (ampAccounts?.active_account_id ?? null) : null,
              auggie_accounts: id === "auggie" ? (auggieAccounts?.accounts ?? []) : [],
              auggie_active_account_id: id === "auggie" ? (auggieAccounts?.active_account_id ?? null) : null,
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
              || (id === "kiro" && kiroAccountsBusy)
              || (id === "cursor" && cursorAccountsBusy)
              || (id === "amp" && ampAccountsBusy)
              || (id === "auggie" && auggieAccountsBusy);

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
                        const catalogLabel =
                          row.model_catalog_status && row.kind === "api_key"
                            ? row.model_catalog_status
                            : null;
                        const catalogClass =
                          catalogLabel === "ready"
                            ? "settings-pill-ok"
                            : catalogLabel === "manual_only"
                              ? "settings-pill"
                              : catalogLabel === "error"
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
                              {catalogLabel ? (
                                <span className={`settings-pill ${catalogClass}`}>{catalogLabel}</span>
                              ) : null}
                              {row.last_error ? (
                                <span className="settings-pill settings-pill-err" title={row.last_error}>
                                  Error
                                </span>
                              ) : null}
                              {row.model_catalog_error ? (
                                <span className="settings-pill settings-pill-err" title={row.model_catalog_error}>
                                  Models
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
                                      onSelect={() => {
                                        void onRefreshProviderEndpointModels(id, row.endpoint_id!);
                                      }}
                                    >
                                      Refresh models
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
                                  {row.kind === "subscription" && id === "cursor" && row.account_id && row.can_delete ? (
                                    <DropdownMenuItem
                                      className="tw-text-[var(--error-contrast)] focus:tw-bg-[var(--error-soft)]"
                                      onSelect={() => {
                                        const accountId = row.account_id;
                                        if (!accountId) return;
                                        void onCursorDelete(accountId);
                                      }}
                                    >
                                      Delete
                                    </DropdownMenuItem>
                                  ) : null}
                                  {row.kind === "subscription" && id === "amp" && row.account_id && row.can_delete ? (
                                    <DropdownMenuItem
                                      className="tw-text-[var(--error-contrast)] focus:tw-bg-[var(--error-soft)]"
                                      onSelect={() => {
                                        const accountId = row.account_id;
                                        if (!accountId) return;
                                        void onAmpDelete(accountId);
                                      }}
                                    >
                                      Delete
                                    </DropdownMenuItem>
                                  ) : null}
                                  {row.kind === "subscription" && id === "auggie" && row.account_id && row.can_delete ? (
                                    <DropdownMenuItem
                                      className="tw-text-[var(--error-contrast)] focus:tw-bg-[var(--error-soft)]"
                                      onSelect={() => {
                                        const accountId = row.account_id;
                                        if (!accountId) return;
                                        void onAuggieDelete(accountId);
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
        </>
      ) : null}

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
                        subscription_auth_url: null,
                      });
                      if (shouldAutoStartSubscriptionFlow(harnessAuthModal.provider_id)) {
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
                        api_key: "",
                        manual_model_ids: "",
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
                        subscription_auth_url: null,
                        base_url: modalRequiresBaseUrl
                          ? (getHarnessEndpointProviderPreset(harnessAuthModal.endpoint_provider_id).base_url
                            ?? harnessAuthModal.base_url)
                          : "",
                      })}
                    disabled={!modalSupportsApiKey}
                    title={
                      modalSupportsApiKey
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
                      ? "Sign in with Claude in your browser. We capture the token automatically. If that fails, paste the token below."
                      : harnessAuthModal.provider_id === "gemini"
                        ? "Sign in with Google to capture managed Gemini OAuth credentials automatically."
                        : harnessAuthModal.provider_id === "amp"
                          ? "Sign in with Amp in your browser to complete OAuth on this host."
                          : harnessAuthModal.provider_id === "auggie"
                            ? "Sign in with Auggie in your browser to complete OAuth on this host."
                        : harnessAuthModal.provider_id === "kimi"
                          ? "Sign in with Kimi to capture managed credentials automatically."
                          : harnessAuthModal.provider_id === "copilot"
                            ? "Sign in with GitHub to capture managed Copilot auth automatically."
                            : harnessAuthModal.provider_id === "kiro"
                              ? "Paste the Kiro auth token JSON for a managed token cache."
                              : harnessAuthModal.provider_id === "cursor"
                                ? "Sign in with Cursor on this host (unmanaged)."
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
                      Token (optional fallback)
                      <input
                        className="settings-control"
                        value={harnessAuthModal.subscription_token}
                        onChange={(e) => patchHarnessAuthModal({ subscription_token: e.target.value })}
                        onKeyDown={(e) => {
                          if (!shouldSubmitClaudeFallbackOnEnter(harnessAuthModal, e.key)) return;
                          e.preventDefault();
                          void submitHarnessSubscriptionModal();
                        }}
                        placeholder="sk-ant-oat..."
                        type="password"
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
                  </>
                ) : null}
                {harnessAuthModal.provider_id === "copilot" ? (
                  harnessAuthModal.subscription_device_code ? (
                    <label className="settings-harness-modal-label">
                      GitHub device code
                      <input
                        className="settings-control"
                        value={harnessAuthModal.subscription_device_code}
                        readOnly
                        onFocus={(event) => event.currentTarget.select()}
                      />
                    </label>
                  ) : null
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
                {harnessAuthModal.subscription_status || modalSubscriptionAuthUrl ? (
                  <div className="settings-row-desc settings-harness-modal-status">
                    {harnessAuthModal.subscription_status ? (
                      <div>{harnessAuthModal.subscription_status}</div>
                    ) : null}
                    {modalSubscriptionAuthUrl ? (
                      <a
                        className="settings-harness-modal-auth-link"
                        href={modalSubscriptionAuthUrl}
                        target="_blank"
                        rel="noreferrer noopener"
                      >
                        Open sign-in link
                      </a>
                    ) : null}
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
                        subscription_device_code: null,
                        subscription_auth_url: null,
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
                    disabled={!canSubmitSubscriptionModal(harnessAuthModal)}
                  >
                    {subscriptionPrimaryActionLabel(harnessAuthModal)}
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
                      <SelectContent className="tw-z-[1101]">
                        {HARNESS_ENDPOINT_PROVIDER_PRESETS.map((preset) => (
                          <SelectItem key={preset.id} value={preset.id}>
                            {preset.label}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </label>
                ) : null}
                {harnessAuthModal.provider_id === "gemini" ? (
                  <label className="settings-harness-modal-label">
                    Gemini auth mode
                    <Select
                      value={harnessAuthModal.gemini_endpoint_auth_type}
                      onValueChange={(nextAuthType) => {
                        patchHarnessAuthModal({
                          gemini_endpoint_auth_type: nextAuthType === "vertex_ai"
                            ? "vertex_ai"
                            : "gemini_api_key",
                        });
                      }}
                    >
                      <SelectTrigger className="tw-min-w-[10rem]">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent className="tw-z-[1101]">
                        <SelectItem value="gemini_api_key">Gemini API Key</SelectItem>
                        <SelectItem value="vertex_ai">Vertex AI</SelectItem>
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
                  Manual model slugs (optional)
                  <textarea
                    className="settings-control settings-control-wide"
                    value={harnessAuthModal.manual_model_ids}
                    onChange={(e) => patchHarnessAuthModal({ manual_model_ids: e.target.value })}
                    placeholder={"openai/gpt-5.2\nanthropic/claude-sonnet-4.5"}
                    rows={4}
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
                        manual_model_ids: "",
                        subscription_status: null,
                        subscription_auth_url: null,
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
