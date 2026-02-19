import type { Diagnostics, ResourceUtilization, TelemetrySummaryResponse } from "@ctx/types";
import { apiAny } from "./clientBase";

export type Health = {
  version: string;
  daemon_version: string;
  pid: number;
  data_root: string;
  daemon_url: string;
  auth_required: boolean;
  compatibility: {
    desktop_exact_version: string;
    mobile_api_min: number;
    mobile_api_max: number;
  };
};

export type UpdateCheck = {
  channel: string;
  base_url: string;
  platform?: string | null;
  current_version: string;
  latest_version?: string | null;
  platform_supported?: boolean;
  update_available: boolean;
  manifest?: unknown;
};

export type DownloadAppImageUpdateResp = {
  downloaded_path: string;
  can_apply_in_place: boolean;
};

export type ApplyAppImageUpdateResp = {
  applied: boolean;
  target_path?: string | null;
  message: string;
};

export type LspServerStatus = {
  language: string;
  command: string;
  args: string[];
  found: boolean;
  resolved_path?: string | null;
  version?: string | null;
  install_hints: string[];
};

export type LspStatus = {
  enabled: boolean;
  edit_plans_enabled: boolean;
  servers: LspServerStatus[];
};

export type LiveKitDictationSettings = {
  base_url: string;
  api_key: string;
  api_secret?: string | null;
  api_secret_set?: boolean;
  model?: string | null;
  language?: string | null;
};

export type DictationSettings = {
  enabled: boolean;
  provider: "disabled" | "livekit_inference" | "tauri_stt";
  livekit?: LiveKitDictationSettings | null;
};

export type TelemetrySettings = {
  enabled: boolean;
  endpoint: string;
};

export type ProviderControlMode = "full" | "harness_native" | "ctx_enforced";

export type SandboxingSettings = {
  provider_control_mode: ProviderControlMode;
};

export type ResourceGovernanceStatusState = "disabled" | "applied" | "pending" | "unsupported" | "error";

export type ResourceGovernanceStatus = {
  state: ResourceGovernanceStatusState;
  can_apply_now: boolean;
  requires_restart: boolean;
  message?: string | null;
};

export type ResourceGovernanceLimits = {
  cpu_quota_pct: number;
  memory_high_mb: number;
  memory_max_mb: number;
};

export type ResourceGovernanceSettings = {
  enabled: boolean;
  mode: "auto" | "custom";
  cpu_quota_pct?: number | null;
  memory_high_mb?: number | null;
  memory_max_mb?: number | null;
  effective?: ResourceGovernanceLimits | null;
  status?: ResourceGovernanceStatus | null;
};

export type ProviderGuardSettings = {
  enabled: boolean;
  mode: "auto" | "custom";
  memory_high_mb?: number | null;
  memory_max_mb?: number | null;
  interval_ms?: number | null;
  grace_period_ms?: number | null;
};

export type SubagentSettings = {
  max_per_call?: number | null;
};

export type TitleGenerationMode = "remote" | "local";

export type TitleGenerationRemoteSettings = {
  base_url: string;
  api_key: string;
  model: string;
  use_json: boolean;
};

export type TitleGenerationLocalSettings = {
  model_id: string;
  use_json: boolean;
};

export type TitleGenerationSettings = {
  mode: TitleGenerationMode;
  remote: TitleGenerationRemoteSettings;
  local: TitleGenerationLocalSettings;
};

export type TitleGenerationLocalRuntimeStatus = {
  version: string;
  installed: boolean;
  path?: string | null;
};

export type TitleGenerationLocalModelStatus = {
  model_id: string;
  file_name: string;
  installed: boolean;
  version?: string | null;
  sha256?: string | null;
  size_bytes?: number | null;
  installed_at?: string | null;
};

export type TitleGenerationLocalStatus = {
  ready: boolean;
  runtime: TitleGenerationLocalRuntimeStatus;
  model: TitleGenerationLocalModelStatus;
  install_id?: string | null;
  install_running?: boolean;
};

export type Settings = {
  dictation?: DictationSettings | null;
  telemetry?: TelemetrySettings | null;
  title_generation?: TitleGenerationSettings | null;
  resource_governance?: ResourceGovernanceSettings | null;
  provider_guard?: ProviderGuardSettings | null;
  subagents?: SubagentSettings | null;
  sandboxing?: SandboxingSettings | null;
  execution?: ExecutionSettings | null;
  network_profiles?: NetworkProfilesSettings | null;
};

export type ExecutionMode = "host" | "container";
export type ContainerRuntimeKind = "podman";
export type ContainerMountMode = "host_mounted" | "disk_isolated";
export type ContainerNetworkMode = "llm_only" | "allowlist" | "all";

export type ContainerExecutionSettings = {
  runtime: ContainerRuntimeKind;
  mount_mode: ContainerMountMode;
  network_mode: ContainerNetworkMode;
  allowlist: string[];
  image?: string | null;
};

export type ExecutionSettings = {
  mode: ExecutionMode;
  container: ContainerExecutionSettings;
};

export type NetworkProfile = {
  mode: ContainerNetworkMode;
  allowlist: string[];
};

export type NetworkProfilesSettings = {
  agent_default: NetworkProfile;
  merge_queue: NetworkProfile;
  worktree_setup: NetworkProfile;
  user_shell: NetworkProfile;
};

export const getSettings = () =>
  apiAny<Settings>("/api/settings");

export const updateSettings = (settings: Partial<Settings>) =>
  apiAny<Settings>("/api/settings", {
    method: "POST",
    body: JSON.stringify(settings),
  });

export const getTitleGenerationLocalStatus = () =>
  apiAny<TitleGenerationLocalStatus>("/api/title_generation/local/status");

export const installTitleGenerationLocal = () =>
  apiAny<{ install_id: string }>("/api/title_generation/local/install", { method: "POST" });

export const getDiagnostics = () => apiAny<Diagnostics>(`/api/diagnostics`);

export const getTelemetrySummary = (params?: { metric?: string; run_id?: string; window_ms?: number; limit?: number }) => {
  const search = new URLSearchParams();
  if (params?.metric) search.set("metric", params.metric);
  if (params?.run_id) search.set("run_id", params.run_id);
  if (params?.window_ms) search.set("window_ms", String(params.window_ms));
  if (params?.limit) search.set("limit", String(params.limit));
  const suffix = search.toString() ? `?${search.toString()}` : "";
  return apiAny<TelemetrySummaryResponse>(`/api/telemetry/summary${suffix}`);
};

export const getResourceUtilization = (workspaceId: string) =>
  apiAny<ResourceUtilization>(`/api/resource_utilization?workspace_id=${encodeURIComponent(workspaceId)}`);

export const getHealth = () => apiAny<Health>(`/api/health`);

export const getLspStatus = () => apiAny<LspStatus>(`/api/lsp/status`);

export const openLogsFolder = () => apiAny(`/api/logs/open`, { method: "POST" });

export const appendDesktopLog = (message: string, level?: string) =>
  apiAny(`/api/desktop/log`, {
    method: "POST",
    body: JSON.stringify({ message, level }),
  });

export const checkUpdates = (channel?: string) =>
  apiAny<UpdateCheck>(`/api/updates/check${channel ? `?channel=${encodeURIComponent(channel)}` : ""}`);

export const downloadAppImageUpdate = (channel?: string) =>
  apiAny<DownloadAppImageUpdateResp>(`/api/updates/appimage/download`, {
    method: "POST",
    body: JSON.stringify(channel ? { channel } : {}),
  });

export const applyAppImageUpdate = () =>
  apiAny<ApplyAppImageUpdateResp>(`/api/updates/appimage/apply`, {
    method: "POST",
    body: JSON.stringify({ confirm: true }),
  });
