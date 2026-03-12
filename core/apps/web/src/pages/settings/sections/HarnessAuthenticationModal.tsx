import { KeyRound, User as UserIcon, X } from "lucide-react";
import { ExternalLink } from "../../../components/ExternalLink";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "../../../components/ui/select";
import type { HarnessAuthModalState } from "../../SettingsPage.types";
import {
  getHarnessEndpointProviderPreset,
  HARNESS_ENDPOINT_PROVIDER_PRESETS,
  type HarnessEndpointProviderPreset,
  supportsOptionalBaseUrlForHarness,
} from "../harnessEndpointProviders";

export type HarnessAuthenticationModalHarness = {
  label: string;
  logoSrc: string;
  invertInDark?: boolean;
  invertInLight?: boolean;
};

type HarnessAuthenticationModalProps = {
  harnessAuthModal: HarnessAuthModalState | null;
  activeModalHarness?: HarnessAuthenticationModalHarness;
  closeHarnessAuthModal: () => void;
  patchHarnessAuthModal: (patch: Partial<HarnessAuthModalState>) => void;
  submitHarnessSubscriptionModal: () => Promise<void>;
  submitHarnessApiKeyModal: () => Promise<void>;
  supportsHarnessEndpointConfig: (providerId: string) => boolean;
  supportsHarnessSubscriptionAuth: (providerId: string) => boolean;
  harnessEndpointRequiresBaseUrl: (providerId: string) => boolean;
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
    || modal.provider_id === "auggie"
    || modal.provider_id === "amp"
    || modal.provider_id === "cursor"
    || modal.provider_id === "gemini"
    || modal.provider_id === "qwen"
    || modal.provider_id === "mistral"
    || (modal.provider_id === "claude-crp" && !claudeSetupTokenProvided(modal))
  ) {
    return modal.provider_id === "gemini" ? "Sign in with Google" : "Start sign-in";
  }
  if (modal.provider_id === "kimi") {
    return "Sign in with Kimi";
  }
  if (modal.provider_id === "copilot") {
    return "Sign in with GitHub";
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
    || providerId === "qwen"
    || providerId === "cursor"
    || providerId === "amp"
    || providerId === "mistral"
    || providerId === "copilot"
    || providerId === "auggie";
}

export function HarnessAuthenticationModal({
  harnessAuthModal,
  activeModalHarness,
  closeHarnessAuthModal,
  patchHarnessAuthModal,
  submitHarnessSubscriptionModal,
  submitHarnessApiKeyModal,
  supportsHarnessEndpointConfig,
  supportsHarnessSubscriptionAuth,
  harnessEndpointRequiresBaseUrl,
}: HarnessAuthenticationModalProps) {
  if (!harnessAuthModal) return null;

  const activeModalEndpointPreset = getHarnessEndpointProviderPreset(harnessAuthModal.endpoint_provider_id);
  const modalRequiresBaseUrl = harnessEndpointRequiresBaseUrl(harnessAuthModal.provider_id);
  const modalProviderUsesNativeKeyFlow =
    harnessAuthModal.provider_id === "cursor" || harnessAuthModal.provider_id === "gemini";
  const modalAllowsCustomBaseUrl = activeModalEndpointPreset.id === "other";
  const modalAllowsOptionalBaseUrl = supportsOptionalBaseUrlForHarness(harnessAuthModal.provider_id);
  const showBaseUrlInput =
    (modalRequiresBaseUrl && modalAllowsCustomBaseUrl)
    || (!modalRequiresBaseUrl && modalAllowsOptionalBaseUrl);
  const modalSupportsApiKey =
    harnessAuthModal.provider_id === "cursor"
    || supportsHarnessEndpointConfig(harnessAuthModal.provider_id);
  const modalSupportsSubscription = supportsHarnessSubscriptionAuth(harnessAuthModal.provider_id);
  const modalUsesGeminiVertexServiceAccount = harnessAuthModal.provider_id === "gemini"
    && harnessAuthModal.gemini_endpoint_auth_type === "vertex_ai";
  const modalApiKeyLabel = harnessAuthModal.provider_id === "gemini"
    ? modalUsesGeminiVertexServiceAccount
      ? "Service account JSON"
      : "Gemini API key"
    : "API key";
  const modalEndpointNameLabel = modalProviderUsesNativeKeyFlow ? "Label (optional)" : "Name (optional)";
  const modalApiKeyPlaceholder = harnessAuthModal.provider_id === "gemini"
    ? modalUsesGeminiVertexServiceAccount
      ? '{"type":"service_account","project_id":"my-project",...}'
      : "AIza..."
    : harnessAuthModal.provider_id === "cursor"
      ? "key_..."
      : harnessAuthModal.provider_id === "auggie"
        ? "Auggie session token"
        : "sk-...";

  const renderEndpointProviderIdentity = (preset: HarnessEndpointProviderPreset) => (
    <span className="settings-endpoint-provider-option">
      {preset.logo_src ? (
        <img
          className={`settings-endpoint-provider-logo ${preset.invert_in_dark ? "wb-invert" : ""} ${
            preset.invert_in_light ? "wb-invert-light" : ""
          }`}
          src={preset.logo_src}
          alt=""
        />
      ) : (
        <span className="settings-endpoint-provider-logo-fallback" aria-hidden="true" />
      )}
      <span className="settings-endpoint-provider-label">{preset.label}</span>
    </span>
  );

  return (
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
              {modalSupportsSubscription ? (
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
                    if (shouldAutoStartSubscriptionFlow(harnessAuthModal.provider_id)) {
                      void submitHarnessSubscriptionModal();
                    }
                  }}
                  disabled={harnessAuthModal.subscription_busy || harnessAuthModal.api_key_busy}
                >
                  <UserIcon size={18} className="settings-harness-modal-choice-icon" aria-hidden="true" />
                  <span>Subscription</span>
                </button>
              ) : null}
              {modalSupportsApiKey ? (
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
                      base_url: modalRequiresBaseUrl
                        ? getHarnessEndpointProviderPreset(harnessAuthModal.endpoint_provider_id).base_url
                          ?? harnessAuthModal.base_url
                        : "",
                    })}
                  title="Add API key auth"
                >
                  <KeyRound size={18} className="settings-harness-modal-choice-icon" aria-hidden="true" />
                  <span>API Key</span>
                </button>
              ) : null}
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
                    : harnessAuthModal.provider_id === "qwen"
                      ? "Sign in with Qwen in your browser to capture managed OAuth credentials automatically."
                      : harnessAuthModal.provider_id === "amp"
                        ? "Sign in with Amp in your browser to complete OAuth on this host."
                        : harnessAuthModal.provider_id === "mistral"
                          ? "Sign in with Mistral in your browser to complete managed OAuth on this host."
                          : harnessAuthModal.provider_id === "kimi"
                            ? "Paste Kimi credentials JSON for a managed Kimi share directory."
                          : harnessAuthModal.provider_id === "copilot"
                              ? "Paste a GitHub token with Copilot entitlement for the managed Copilot account."
                              : harnessAuthModal.provider_id === "cursor"
                                ? "Sign in with Cursor in your browser to capture a managed auth token automatically."
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
            {harnessAuthModal.subscription_status ? (
              <div className="settings-row-desc settings-harness-modal-status">
                {harnessAuthModal.subscription_status}
              </div>
            ) : null}
            <div className="modal-actions settings-harness-modal-actions">
              <button
                type="button"
                className="settings-btn settings-btn-secondary"
                onClick={() => {
                  if (!modalSupportsApiKey) {
                    closeHarnessAuthModal();
                    return;
                  }
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
                  });
                }}
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
            {harnessAuthModal.provider_id === "cursor" ? (
              <div className="settings-row-desc">
                Get your Cursor API key from{" "}
                <ExternalLink
                  className="settings-harness-help-link"
                  href="https://cursor.com/dashboard?tab=integrations"
                >
                  Cursor Integrations
                </ExternalLink>
                .
              </div>
            ) : null}
            {harnessAuthModal.provider_id === "gemini" ? (
              <div className="settings-row-desc">
                Create Gemini keys in{" "}
                <ExternalLink
                  className="settings-harness-help-link"
                  href="https://aistudio.google.com/app/apikey"
                >
                  Google AI Studio
                </ExternalLink>
                . For Vertex AI service accounts, use{" "}
                <ExternalLink
                  className="settings-harness-help-link"
                  href="https://console.cloud.google.com/apis/credentials"
                >
                  Google Cloud Credentials
                </ExternalLink>
                .
              </div>
            ) : null}
            {modalRequiresBaseUrl && !modalProviderUsesNativeKeyFlow ? (
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
                  <SelectTrigger className="tw-min-w-[10rem] [&>span]:!tw-inline-flex [&>span]:!tw-items-center [&>span]:!tw-gap-2 [&>span]:!tw-line-clamp-none">
                    {renderEndpointProviderIdentity(
                      getHarnessEndpointProviderPreset(harnessAuthModal.endpoint_provider_id),
                    )}
                  </SelectTrigger>
                  <SelectContent className="tw-z-[1101]">
                    {HARNESS_ENDPOINT_PROVIDER_PRESETS.map((preset) => (
                      <SelectItem key={preset.id} value={preset.id}>
                        {renderEndpointProviderIdentity(preset)}
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
                    const nextProviderId = nextAuthType === "vertex_ai" ? "google_vertex" : "google_ai_studio";
                    const nextPreset = getHarnessEndpointProviderPreset(nextProviderId);
                    patchHarnessAuthModal({
                      gemini_endpoint_auth_type: nextAuthType === "vertex_ai"
                        ? "vertex_ai"
                        : "gemini_api_key",
                      endpoint_provider_id: nextProviderId,
                      base_url: nextPreset.base_url ?? "",
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
            {modalUsesGeminiVertexServiceAccount ? (
              <>
                <label className="settings-harness-modal-label">
                  {modalApiKeyLabel}
                  <textarea
                    className="settings-control settings-control-wide"
                    placeholder={modalApiKeyPlaceholder}
                    value={harnessAuthModal.service_account_json}
                    onChange={(e) => patchHarnessAuthModal({ service_account_json: e.target.value })}
                    rows={8}
                  />
                </label>
                <label className="settings-harness-modal-label">
                  Project ID (optional)
                  <input
                    className="settings-control settings-control-wide"
                    placeholder="my-gcp-project"
                    value={harnessAuthModal.project_id}
                    onChange={(e) => patchHarnessAuthModal({ project_id: e.target.value })}
                  />
                </label>
                <label className="settings-harness-modal-label">
                  Location (optional)
                  <input
                    className="settings-control settings-control-wide"
                    placeholder="global"
                    value={harnessAuthModal.location}
                    onChange={(e) => patchHarnessAuthModal({ location: e.target.value })}
                  />
                </label>
              </>
            ) : (
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
            )}
            {!modalProviderUsesNativeKeyFlow ? (
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
            ) : null}
            <label className="settings-harness-modal-label">
              {modalEndpointNameLabel}
              <input
                className="settings-control settings-control-wide"
                value={harnessAuthModal.endpoint_name}
                onChange={(e) => patchHarnessAuthModal({ endpoint_name: e.target.value })}
              />
            </label>
            {showBaseUrlInput && !modalProviderUsesNativeKeyFlow ? (
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
                onClick={() => {
                  if (!modalSupportsSubscription) {
                    closeHarnessAuthModal();
                    return;
                  }
                  patchHarnessAuthModal({
                    stage: "choose",
                    api_key: "",
                    service_account_json: "",
                    project_id: "",
                    location: "",
                    manual_model_ids: "",
                    subscription_status: null,
                  });
                }}
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
  );
}
