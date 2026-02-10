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
  auth_methods?: any;
  modes?: any;
  models?: any;
  verify?: any;
  probed_at: string;
};

export const getProviderOptions = (workspaceId: string, providerId: string) =>
  apiAny<ProviderOptions>(`/api/workspaces/${workspaceId}/providers/${providerId}/options`);

export type ProviderAuthCheck = {
  provider_id: string;
  workspace_id: string;
  status: string;
  auth_required?: boolean;
  auth_methods?: any;
  checked_at?: string;
};

export type ProviderUsageSnapshot = {
  provider_id: string;
  source: string;
  fetched_at: string;
  payload?: any;
  error?: string;
};

export const authenticateProviderForWorkspace = (workspaceId: string, providerId: string, method_id?: string) =>
  apiAny<ProviderAuthCheck>(`/api/workspaces/${workspaceId}/providers/${providerId}/authenticate`, {
    method: "POST",
    body: JSON.stringify(method_id ? { method_id } : {}),
  });

export const verifyProviderForWorkspace = (workspaceId: string, providerId: string) =>
  apiAny<ProviderAuthCheck>(`/api/workspaces/${workspaceId}/providers/${providerId}/verify`, { method: "POST" });

export const getProviderUsage = (providerId: string, refresh?: boolean) => {
  const params = refresh ? "?refresh=true" : "";
  return apiAny<ProviderUsageSnapshot>(`/api/providers/${providerId}/usage${params}`);
};

export type CodexAccountEntry = {
  id: string;
  label: string;
  email?: string | null;
  plan_type?: string | null;
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
  status: string;
  error?: string | null;
};

export type CodexAccountsResponse = {
  active_account_id: string | null;
  accounts: CodexAccountEntry[];
  logins: CodexLoginStatus[];
};

export type CodexLoginStartResponse = {
  account_id: string;
  auth_url: string;
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

export const setCodexActiveAccount = (accountId: string | null) =>
  apiAny<CodexAccountsResponse>(`/api/providers/codex/active-account`, {
    method: "PUT",
    body: JSON.stringify({ account_id: accountId }),
  });

export const deleteCodexAccount = (accountId: string) =>
  apiAny<CodexAccountsResponse>(`/api/providers/codex/accounts/${accountId}`, {
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
