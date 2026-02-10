import type { InstallInfo } from "../api/client";

export type SectionId =
  | "general"
  | "privacy"
  | "agent_harnesses"
  | "credential_imports"
  | "harness_subscriptions"
  | "models_routing"
  | "sandboxing"
  | "execution"
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
