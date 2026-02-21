import type { ProviderStatus } from "@ctx/types";
import { apiAny, authToken } from "./clientBase";

export type InstallEventLevel = "info" | "warning" | "error" | "success";

export type InstallProgressEvent = {
  install_id: string;
  provider_id: string;
  at: string;
  stage: string;
  message: string;
  level: InstallEventLevel;
  bytes?: number;
  total_bytes?: number;
  attempt?: number;
};

export type InstallInfo = {
  install_id: string;
  provider_id: string;
  state: "running" | "succeeded" | "failed";
  started_at: string;
  finished_at?: string;
  error?: string;
  last_event?: InstallProgressEvent;
};

export type InstallStartResponse = {
  provider_id: string;
  install_id: string;
};

export const listProviders = () =>
  apiAny<ProviderStatus[]>(`/api/providers`);

export type ProviderOptions = {
  provider_id: string;
  workspace_id: string;
  installed?: boolean;
  probe_ok?: boolean;
  probe_error?: string;
  supports_load: boolean;
  auth_required: boolean;
  has_active_auth?: boolean;
  auth_mode?: "subscription" | "endpoint" | "none";
  auth_methods?: unknown;
  modes?: unknown;
  models?: unknown;
  verify?: unknown;
  source?: HarnessProviderSourceConfig;
  probed_at: string;
};

export const getProviderOptions = (workspaceId: string, providerId: string) =>
  apiAny<ProviderOptions>(`/api/workspaces/${workspaceId}/providers/${providerId}/options`);

export type ProvidersBootstrapResponse = {
  providers: ProviderStatus[];
  provider_options: Record<string, ProviderOptions>;
  provider_harness_config: Record<string, HarnessProviderSourceConfig>;
  codex_accounts: CodexAccountsResponse;
  claude_accounts: ClaudeAccountsResponse;
  gemini_accounts: GeminiAccountsResponse;
  kimi_accounts: KimiAccountsResponse;
  copilot_accounts: CopilotAccountsResponse;
  kiro_accounts: KiroAccountsResponse;
  cursor_accounts: CursorAccountsResponse;
  amp_accounts: AmpAccountsResponse;
};

export const getProvidersBootstrap = (workspaceId: string) =>
  apiAny<ProvidersBootstrapResponse>(`/api/workspaces/${workspaceId}/providers/bootstrap`);

export type ProviderAuthCheck = {
  provider_id: string;
  workspace_id: string;
  status: string;
  auth_required?: boolean;
  auth_methods?: unknown;
  checked_at?: string;
  message?: string;
};

export type HarnessSourceKind = "subscription" | "endpoint";
export type HarnessApiShape = "openai_responses" | "anthropic_messages";
export type HarnessEndpointVerificationStatus = "unknown" | "valid" | "invalid" | "error";
export type EndpointModelCatalogStatus = "unknown" | "ready" | "manual_only" | "error";

export type EndpointModelRecord = {
  id: string;
  name?: string | null;
};

export type HarnessEndpointRecord = {
  id: string;
  provider_id: string;
  name: string;
  base_url?: string | null;
  api_shape: HarnessApiShape;
  auth_type: string;
  model_override?: string | null;
  created_at: string;
  updated_at: string;
  last_verification_status: HarnessEndpointVerificationStatus;
  last_verification_at?: string | null;
  last_error?: string | null;
  has_api_key: boolean;
  model_catalog_status?: EndpointModelCatalogStatus;
  model_catalog_fetched_at?: string | null;
  model_catalog_error?: string | null;
  model_catalog_models?: EndpointModelRecord[];
  manual_model_ids?: string[];
  model_catalog_source?: string | null;
};

export type HarnessProviderSourceConfig = {
  provider_id: string;
  selected_source_kind: HarnessSourceKind;
  selected_endpoint_id?: string | null;
  endpoints: HarnessEndpointRecord[];
};

export type UpsertHarnessEndpointRequest = {
  endpoint_id?: string | null;
  name: string;
  base_url?: string | null;
  api_shape?: HarnessApiShape | null;
  auth_type?: string | null;
  model_override?: string | null;
  api_key?: string | null;
  manual_model_ids?: string[] | null;
};

export type ProviderUsageSnapshot = {
  provider_id: string;
  source: string;
  fetched_at: string;
  payload?: unknown;
  error?: string;
};

export const getProviderUsage = (providerId: string, refresh?: boolean) => {
  const params = refresh ? "?refresh=true" : "";
  return apiAny<ProviderUsageSnapshot>(`/api/providers/${providerId}/usage${params}`);
};

export const getProviderHarnessConfig = (providerId: string) =>
  apiAny<HarnessProviderSourceConfig>(`/api/providers/${providerId}/harness_config`);

export const selectProviderHarnessSource = (
  providerId: string,
  sourceKind: HarnessSourceKind,
  endpointId?: string | null,
) =>
  apiAny<HarnessProviderSourceConfig>(`/api/providers/${providerId}/harness_config/select`, {
    method: "POST",
    body: JSON.stringify({
      source_kind: sourceKind,
      endpoint_id: endpointId ?? null,
    }),
  });

export const upsertProviderHarnessEndpoint = (providerId: string, req: UpsertHarnessEndpointRequest) =>
  apiAny<HarnessProviderSourceConfig>(`/api/providers/${providerId}/harness_config/endpoints`, {
    method: "POST",
    body: JSON.stringify(req),
  });

export const deleteProviderHarnessEndpoint = (providerId: string, endpointId: string) =>
  apiAny<HarnessProviderSourceConfig>(`/api/providers/${providerId}/harness_config/endpoints/${endpointId}`, {
    method: "DELETE",
  });

export const refreshProviderHarnessEndpointModels = (providerId: string, endpointId: string) =>
  apiAny<HarnessProviderSourceConfig>(`/api/providers/${providerId}/harness_config/endpoints/${endpointId}/models/refresh`, {
    method: "POST",
    body: JSON.stringify({}),
  });

export const setProviderHarnessEndpointManualModels = (
  providerId: string,
  endpointId: string,
  modelIds: string[],
) =>
  apiAny<HarnessProviderSourceConfig>(`/api/providers/${providerId}/harness_config/endpoints/${endpointId}/models/manual`, {
    method: "PUT",
    body: JSON.stringify({ model_ids: modelIds }),
  });

export const authenticateProviderForWorkspace = (
  workspaceId: string,
  providerId: string,
  methodId?: string,
) =>
  apiAny<ProviderAuthCheck>(`/api/workspaces/${workspaceId}/providers/${providerId}/authenticate`, {
    method: "POST",
    body: JSON.stringify(methodId ? { method_id: methodId } : {}),
  });

export const verifyProviderForWorkspace = (workspaceId: string, providerId: string) =>
  apiAny<ProviderAuthCheck>(`/api/workspaces/${workspaceId}/providers/${providerId}/verify`, {
    method: "POST",
    body: JSON.stringify({}),
  });

export type CodexAccountEntry = {
  id: string;
  label: string;
  kind?: string;
  email?: string | null;
  plan_type?: string | null;
  created_at: string;
  last_used_at?: string | null;
};

export type ClaudeAccountEntry = {
  id: string;
  label: string;
  kind?: string;
  email?: string | null;
  subscription_type?: string | null;
  created_at: string;
  last_used_at?: string | null;
};

export type GeminiAccountEntry = {
  id: string;
  label: string;
  kind?: string;
  email?: string | null;
  created_at: string;
  last_used_at?: string | null;
};

export type KimiAccountEntry = {
  id: string;
  label: string;
  kind?: string;
  email?: string | null;
  created_at: string;
  last_used_at?: string | null;
};

export type CopilotAccountEntry = {
  id: string;
  label: string;
  kind?: string;
  email?: string | null;
  created_at: string;
  last_used_at?: string | null;
};

export type KiroAccountEntry = {
  id: string;
  label: string;
  kind?: string;
  email?: string | null;
  created_at: string;
  last_used_at?: string | null;
};

export type CursorAccountEntry = {
  id: string;
  label: string;
  kind?: string;
  email?: string | null;
  created_at: string;
  last_used_at?: string | null;
};

export type AmpAccountEntry = {
  id: string;
  label: string;
  kind?: string;
  email?: string | null;
  created_at: string;
  last_used_at?: string | null;
};

export type CodexAccountUsageEntry = {
  account_id: string | null;
  label: string;
  email?: string | null;
  plan_type?: string | null;
  last_used_at?: string | null;
  usage: ProviderUsageSnapshot;
};

export type CodexAccountUsageResponse = {
  entries: CodexAccountUsageEntry[];
};

export type CodexLoginStatus = {
  account_id: string;
  auth_url: string;
  expected_callback_url?: string | null;
  completion_token?: string | null;
  status: string;
  error?: string | null;
};

export type CodexAccountsResponse = {
  active_account_id: string | null;
  accounts: CodexAccountEntry[];
  logins: CodexLoginStatus[];
};

export type ClaudeAccountsResponse = {
  active_account_id: string | null;
  accounts: ClaudeAccountEntry[];
};

export type GeminiAccountsResponse = {
  active_account_id: string | null;
  accounts: GeminiAccountEntry[];
};

export type KimiAccountsResponse = {
  active_account_id: string | null;
  accounts: KimiAccountEntry[];
};

export type CopilotAccountsResponse = {
  active_account_id: string | null;
  accounts: CopilotAccountEntry[];
  logins: CopilotLoginStatus[];
};

export type KiroAccountsResponse = {
  active_account_id: string | null;
  accounts: KiroAccountEntry[];
};

export type CursorAccountsResponse = {
  active_account_id: string | null;
  accounts: CursorAccountEntry[];
};

export type AmpAccountsResponse = {
  active_account_id: string | null;
  accounts: AmpAccountEntry[];
};

export type CodexLoginStartResponse = {
  account_id: string;
  auth_url: string;
  expected_callback_url?: string | null;
  completion_token: string;
};

export type CodexLoginCompleteResponse = {
  accepted: boolean;
  status_code: number;
};

export type ClaudeLoginStatus = {
  login_id: string;
  auth_url?: string | null;
  status: string;
  account_id?: string | null;
  error?: string | null;
};

export type ClaudeLoginStartResponse = {
  login_id: string;
  auth_url?: string | null;
};

export type ClaudeLoginCompleteResponse = {
  accepted: boolean;
};

export type GeminiLoginStatus = {
  login_id: string;
  auth_url?: string | null;
  status: string;
  account_id?: string | null;
  error?: string | null;
};

export type GeminiLoginStartResponse = {
  login_id: string;
  auth_url?: string | null;
};

export type AmpLoginStatus = {
  login_id: string;
  auth_url?: string | null;
  status: string;
  error?: string | null;
};

export type AmpLoginStartResponse = {
  login_id: string;
  auth_url?: string | null;
};

export type KimiLoginStatus = {
  login_id: string;
  auth_url?: string | null;
  status: string;
  account_id?: string | null;
  error?: string | null;
};

export type KimiLoginStartResponse = {
  login_id: string;
  auth_url?: string | null;
};

export type CopilotLoginStatus = {
  login_id: string;
  auth_url?: string | null;
  status: string;
  account_id?: string | null;
  error?: string | null;
};

export type CopilotLoginStartResponse = {
  login_id: string;
  auth_url?: string | null;
};

export type CodexHostImportProbe = {
  available: boolean;
  path?: string | null;
  auth_kind?: string | null;
  error?: string | null;
};

export type ProviderAuthImportCandidate = {
  id: string;
  provider_id: string;
  provider_label: string;
  kind: string;
  path: string;
  signal_strength: string;
  confidence: string;
  parse_status: string;
  unsupported_reason?: string | null;
  summary?: string | null;
  account_identity?: string | null;
  endpoint?: string | null;
  auth_type?: string | null;
  fingerprint?: string | null;
  last_modified?: string | null;
};

export type ProviderAuthImportResult = {
  candidate_id: string;
  provider_id: string;
  status: string;
  profile_id?: string | null;
  message?: string | null;
};

export type ProviderImportedAuthProfile = {
  id: string;
  provider_id: string;
  provider_label: string;
  label: string;
  account_identity?: string | null;
  endpoint?: string | null;
  auth_type?: string | null;
  source_path: string;
  source_kind: string;
  secret_fingerprint: string;
  imported_at: string;
  updated_at: string;
};

export const listCodexAccounts = () =>
  apiAny<CodexAccountsResponse>(`/api/providers/codex/accounts`);

export const probeCodexHostImport = () =>
  apiAny<CodexHostImportProbe>(`/api/providers/codex/import/host`);

export const importCodexHostAuth = (label?: string) =>
  apiAny<CodexAccountsResponse>(`/api/providers/codex/import/host`, {
    method: "POST",
    body: JSON.stringify(label ? { label } : {}),
  });

export const getCodexAccountUsage = (refresh?: boolean) => {
  const params = refresh ? "?refresh=true" : "";
  return apiAny<CodexAccountUsageResponse>(`/api/providers/codex/accounts/usage${params}`);
};

export const startCodexLogin = (label?: string) =>
  apiAny<CodexLoginStartResponse>(`/api/providers/codex/accounts/login/start`, {
    method: "POST",
    body: JSON.stringify(label ? { label } : {}),
  });

export const getCodexLogin = (accountId: string) =>
  apiAny<CodexLoginStatus>(`/api/providers/codex/accounts/login/${accountId}`);

export const completeCodexLogin = (accountId: string, callbackUrl: string, completionToken: string) =>
  apiAny<CodexLoginCompleteResponse>(`/api/providers/codex/accounts/login/${accountId}`, {
    method: "POST",
    body: JSON.stringify({
      callback_url: callbackUrl,
      completion_token: completionToken,
    }),
  });

export const setCodexActiveAccount = (accountId: string | null) =>
  apiAny<CodexAccountsResponse>(`/api/providers/codex/active-account`, {
    method: "PUT",
    body: JSON.stringify({ account_id: accountId }),
  });

export const deleteCodexAccount = (accountId: string) =>
  apiAny<CodexAccountsResponse>(`/api/providers/codex/accounts/${accountId}`, {
    method: "DELETE",
  });

export const listClaudeAccounts = () =>
  apiAny<ClaudeAccountsResponse>(`/api/providers/claude-crp/accounts`);

export const startClaudeLogin = (label?: string) =>
  apiAny<ClaudeLoginStartResponse>(`/api/providers/claude-crp/accounts/login/start`, {
    method: "POST",
    body: JSON.stringify(label ? { label } : {}),
  });

export const getClaudeLogin = (loginId: string) =>
  apiAny<ClaudeLoginStatus>(`/api/providers/claude-crp/accounts/login/${loginId}`);

export const completeClaudeLogin = (loginId: string, callbackCode: string) =>
  apiAny<ClaudeLoginCompleteResponse>(`/api/providers/claude-crp/accounts/login/${loginId}`, {
    method: "POST",
    body: JSON.stringify({ callback_code: callbackCode }),
  });

export const upsertClaudeAccount = (setupToken: string, label?: string) =>
  apiAny<ClaudeAccountsResponse>(`/api/providers/claude-crp/accounts`, {
    method: "POST",
    body: JSON.stringify(label ? { setup_token: setupToken, label } : { setup_token: setupToken }),
  });

export const setClaudeActiveAccount = (accountId: string | null) =>
  apiAny<ClaudeAccountsResponse>(`/api/providers/claude-crp/active-account`, {
    method: "PUT",
    body: JSON.stringify({ account_id: accountId }),
  });

export const deleteClaudeAccount = (accountId: string) =>
  apiAny<ClaudeAccountsResponse>(`/api/providers/claude-crp/accounts/${accountId}`, {
    method: "DELETE",
  });

export const listGeminiAccounts = () =>
  apiAny<GeminiAccountsResponse>(`/api/providers/gemini/accounts`);

export const startGeminiLogin = (label?: string) =>
  apiAny<GeminiLoginStartResponse>(`/api/providers/gemini/accounts/login/start`, {
    method: "POST",
    body: JSON.stringify(label ? { label } : {}),
  });

export const getGeminiLogin = (loginId: string) =>
  apiAny<GeminiLoginStatus>(`/api/providers/gemini/accounts/login/${loginId}`);

export const startAmpLogin = (label?: string) =>
  apiAny<AmpLoginStartResponse>(`/api/providers/amp/accounts/login/start`, {
    method: "POST",
    body: JSON.stringify(label ? { label } : {}),
  });

export const getAmpLogin = (loginId: string) =>
  apiAny<AmpLoginStatus>(`/api/providers/amp/accounts/login/${loginId}`);

export const listAmpAccounts = () =>
  apiAny<AmpAccountsResponse>(`/api/providers/amp/accounts`);

export const setAmpActiveAccount = (accountId: string | null) =>
  apiAny<AmpAccountsResponse>(`/api/providers/amp/active-account`, {
    method: "PUT",
    body: JSON.stringify({ account_id: accountId }),
  });

export const deleteAmpAccount = (accountId: string) =>
  apiAny<AmpAccountsResponse>(`/api/providers/amp/accounts/${accountId}`, {
    method: "DELETE",
  });

export const upsertGeminiAccount = (
  oauthCredsJson: string,
  opts?: { label?: string; googleAccountsJson?: string; email?: string },
) =>
  apiAny<GeminiAccountsResponse>(`/api/providers/gemini/accounts`, {
    method: "POST",
    body: JSON.stringify({
      oauth_creds_json: oauthCredsJson,
      ...(opts?.label ? { label: opts.label } : {}),
      ...(opts?.googleAccountsJson ? { google_accounts_json: opts.googleAccountsJson } : {}),
      ...(opts?.email ? { email: opts.email } : {}),
    }),
  });

export const setGeminiActiveAccount = (accountId: string | null) =>
  apiAny<GeminiAccountsResponse>(`/api/providers/gemini/active-account`, {
    method: "PUT",
    body: JSON.stringify({ account_id: accountId }),
  });

export const deleteGeminiAccount = (accountId: string) =>
  apiAny<GeminiAccountsResponse>(`/api/providers/gemini/accounts/${accountId}`, {
    method: "DELETE",
  });

export const listKimiAccounts = () =>
  apiAny<KimiAccountsResponse>(`/api/providers/kimi/accounts`);

export const startKimiLogin = (label?: string) =>
  apiAny<KimiLoginStartResponse>(`/api/providers/kimi/accounts/login/start`, {
    method: "POST",
    body: JSON.stringify(label ? { label } : {}),
  });

export const getKimiLogin = (loginId: string) =>
  apiAny<KimiLoginStatus>(`/api/providers/kimi/accounts/login/${loginId}`);

export const upsertKimiAccount = (
  credentialsJson: string,
  opts?: {
    label?: string;
    provider?: string;
    configToml?: string;
    email?: string;
  },
) =>
  apiAny<KimiAccountsResponse>(`/api/providers/kimi/accounts`, {
    method: "POST",
    body: JSON.stringify({
      credentials_json: credentialsJson,
      ...(opts?.label ? { label: opts.label } : {}),
      ...(opts?.provider ? { provider: opts.provider } : {}),
      ...(opts?.configToml ? { config_toml: opts.configToml } : {}),
      ...(opts?.email ? { email: opts.email } : {}),
    }),
  });

export const setKimiActiveAccount = (accountId: string | null) =>
  apiAny<KimiAccountsResponse>(`/api/providers/kimi/active-account`, {
    method: "PUT",
    body: JSON.stringify({ account_id: accountId }),
  });

export const deleteKimiAccount = (accountId: string) =>
  apiAny<KimiAccountsResponse>(`/api/providers/kimi/accounts/${accountId}`, {
    method: "DELETE",
  });

export const listCopilotAccounts = () =>
  apiAny<CopilotAccountsResponse>(`/api/providers/copilot/accounts`);

export const startCopilotLogin = (label?: string) =>
  apiAny<CopilotLoginStartResponse>(`/api/providers/copilot/accounts/login/start`, {
    method: "POST",
    body: JSON.stringify(label ? { label } : {}),
  });

export const getCopilotLogin = (loginId: string) =>
  apiAny<CopilotLoginStatus>(`/api/providers/copilot/accounts/login/${loginId}`);

export const upsertCopilotAccount = (
  token: string,
  opts?: { label?: string; email?: string },
) =>
  apiAny<CopilotAccountsResponse>(`/api/providers/copilot/accounts`, {
    method: "POST",
    body: JSON.stringify({
      token,
      ...(opts?.label ? { label: opts.label } : {}),
      ...(opts?.email ? { email: opts.email } : {}),
    }),
  });

export const setCopilotActiveAccount = (accountId: string | null) =>
  apiAny<CopilotAccountsResponse>(`/api/providers/copilot/active-account`, {
    method: "PUT",
    body: JSON.stringify({ account_id: accountId }),
  });

export const deleteCopilotAccount = (accountId: string) =>
  apiAny<CopilotAccountsResponse>(`/api/providers/copilot/accounts/${accountId}`, {
    method: "DELETE",
  });

export const listKiroAccounts = () =>
  apiAny<KiroAccountsResponse>(`/api/providers/kiro/accounts`);

export const upsertKiroAccount = (
  authTokenJson: string,
  opts?: { label?: string; email?: string },
) =>
  apiAny<KiroAccountsResponse>(`/api/providers/kiro/accounts`, {
    method: "POST",
    body: JSON.stringify({
      auth_token_json: authTokenJson,
      ...(opts?.label ? { label: opts.label } : {}),
      ...(opts?.email ? { email: opts.email } : {}),
    }),
  });

export const setKiroActiveAccount = (accountId: string | null) =>
  apiAny<KiroAccountsResponse>(`/api/providers/kiro/active-account`, {
    method: "PUT",
    body: JSON.stringify({ account_id: accountId }),
  });

export const deleteKiroAccount = (accountId: string) =>
  apiAny<KiroAccountsResponse>(`/api/providers/kiro/accounts/${accountId}`, {
    method: "DELETE",
  });

export const listCursorAccounts = () =>
  apiAny<CursorAccountsResponse>(`/api/providers/cursor/accounts`);

export const upsertCursorAccount = (
  token: string,
  opts?: { label?: string; email?: string },
) =>
  apiAny<CursorAccountsResponse>(`/api/providers/cursor/accounts`, {
    method: "POST",
    body: JSON.stringify({
      token,
      ...(opts?.label ? { label: opts.label } : {}),
      ...(opts?.email ? { email: opts.email } : {}),
    }),
  });

export const setCursorActiveAccount = (accountId: string | null) =>
  apiAny<CursorAccountsResponse>(`/api/providers/cursor/active-account`, {
    method: "PUT",
    body: JSON.stringify({ account_id: accountId }),
  });

export const deleteCursorAccount = (accountId: string) =>
  apiAny<CursorAccountsResponse>(`/api/providers/cursor/accounts/${accountId}`, {
    method: "DELETE",
  });

export const listProviderAuthImportCandidates = () =>
  apiAny<{ candidates: ProviderAuthImportCandidate[] }>(`/api/providers/auth/import/candidates`);

export const listProviderAuthImportProfiles = () =>
  apiAny<{ profiles: ProviderImportedAuthProfile[] }>(`/api/providers/auth/import/profiles`);

export const importProviderAuthCandidates = (candidateIds: string[]) =>
  apiAny<{ results: ProviderAuthImportResult[] }>(`/api/providers/auth/import`, {
    method: "POST",
    body: JSON.stringify({ candidate_ids: candidateIds }),
  });

export const installProvider = (providerId: string) =>
  apiAny<InstallStartResponse>(`/api/providers/${providerId}/install`, { method: "POST" });

export const installAllProviders = () =>
  apiAny<InstallStartResponse[]>(`/api/providers/install_all`, { method: "POST" });

export const getInstall = (installId: string) =>
  apiAny<InstallInfo>(`/api/providers/install/${installId}`);

export const listInstallEvents = (installId: string) =>
  apiAny<InstallProgressEvent[]>(`/api/providers/install/${installId}/events`);

export const installStreamUrl = (installId: string): string => {
  const token = authToken();
  return token
    ? `/api/providers/install/${installId}/stream?token=${encodeURIComponent(token)}`
    : `/api/providers/install/${installId}/stream`;
};

export type DevRestartProvidersMode = "immediate" | "drain";

export type DevRestartProvidersResult = {
  provider_id: string;
  status: string;
  message?: string;
};

export type DevRestartProvidersResponse = {
  mode: DevRestartProvidersMode;
  results: DevRestartProvidersResult[];
};

export const devRestartProviders = (mode: DevRestartProvidersMode) =>
  apiAny<DevRestartProvidersResponse>(`/api/dev/providers/restart`, {
    method: "POST",
    body: JSON.stringify({ mode }),
  });
