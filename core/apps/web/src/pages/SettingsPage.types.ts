import type { InstallInfo } from "../api/client";

export type SectionId =
  | "general"
  | "notifications"
  | "analytics"
  | "agent_harnesses"
  | "harness_subscriptions"
  | "models_routing"
  | "sandboxing"
  | "container_network"
  | "worktree_bootstrap"
  | "agent_system_prompt"
  | "workspace_attachments"
  | "merge_queue"
  | "context_pack"
  | "resource_governance"
  | "mobile_access"
  | "resource_utilization"
  | "dictation"
  | "title_generation"
  | "billing"
  | "team_enterprise"
  | "usage_analytics"
  | "dev_tools";

export type InstallSession = {
  installId: string;
  state: InstallInfo["state"];
  pct: number | null;
  streamError?: string;
  error?: string;
};

export type SettingsSectionGroup = "main" | "advanced";

export type SettingsSectionMeta = {
  id: SectionId;
  label: string;
  group?: SettingsSectionGroup;
  navHidden?: boolean;
};

export type SettingsSectionComponentId =
  | "general"
  | "notifications"
  | "analytics"
  | "harness_authentication"
  | "container_network"
  | "worktree_bootstrap"
  | "agent_system_prompt"
  | "workspace_attachments"
  | "merge_queue"
  | "dictation"
  | "title_generation"
  | "dev_tools"
  | "legacy";

export type HarnessAuthModalState = {
  provider_id: string;
  stage: "choose" | "subscription" | "api_key";
  endpoint_provider_id: string;
  endpoint_name: string;
  base_url: string;
  api_key: string;
  subscription_status: string | null;
  subscription_busy: boolean;
  api_key_busy: boolean;
};
