import type {
  AuggieAccountEntry,
  AmpAccountEntry,
  ClaudeAccountEntry,
  CopilotAccountEntry,
  CodexAccountEntry,
  CursorAccountEntry,
  GeminiAccountEntry,
  HarnessEndpointRecord,
  HarnessSourceKind,
  KimiAccountEntry,
  KiroAccountEntry,
} from "../../api/client";

export type HarnessAuthRow = {
  key: string;
  kind: "subscription" | "api_key";
  label: string;
  detail?: string;
  active: boolean;
  selectable: boolean;
  account_id?: string;
  endpoint_id?: string;
  can_delete?: boolean;
  verification_status?: string;
  last_error?: string | null;
  model_catalog_status?: string;
  model_catalog_error?: string | null;
  model_count?: number;
  model_catalog_fetched_at?: string | null;
};

type BuildHarnessAuthRowsArgs = {
  provider_id: string;
  selected_source_kind: HarnessSourceKind;
  selected_endpoint_id?: string | null;
  endpoints: HarnessEndpointRecord[];
  codex_accounts: CodexAccountEntry[];
  codex_active_account_id?: string | null;
  claude_accounts: ClaudeAccountEntry[];
  claude_active_account_id?: string | null;
  gemini_accounts: GeminiAccountEntry[];
  gemini_active_account_id?: string | null;
  kimi_accounts: KimiAccountEntry[];
  kimi_active_account_id?: string | null;
  copilot_accounts: CopilotAccountEntry[];
  copilot_active_account_id?: string | null;
  kiro_accounts: KiroAccountEntry[];
  kiro_active_account_id?: string | null;
  cursor_accounts: CursorAccountEntry[];
  cursor_active_account_id?: string | null;
  amp_accounts?: AmpAccountEntry[];
  amp_active_account_id?: string | null;
  auggie_accounts?: AuggieAccountEntry[];
  auggie_active_account_id?: string | null;
};

const codexAccountLabel = (account: CodexAccountEntry): string => {
  if (account.email && account.email.trim()) return account.email.trim();
  if (account.label.trim()) return account.label.trim();
  return account.id;
};

const claudeAccountLabel = (account: ClaudeAccountEntry): string => {
  if (account.email && account.email.trim()) return account.email.trim();
  if (account.label.trim()) return account.label.trim();
  return account.id;
};

const geminiAccountLabel = (account: GeminiAccountEntry): string => {
  if (account.email && account.email.trim()) return account.email.trim();
  if (account.label.trim()) return account.label.trim();
  return account.id;
};

const kimiAccountLabel = (account: KimiAccountEntry): string => {
  if (account.email && account.email.trim()) return account.email.trim();
  if (account.label.trim()) return account.label.trim();
  return account.id;
};

const copilotAccountLabel = (account: CopilotAccountEntry): string => {
  if (account.email && account.email.trim()) return account.email.trim();
  if (account.label.trim()) return account.label.trim();
  return account.id;
};

const kiroAccountLabel = (account: KiroAccountEntry): string => {
  if (account.email && account.email.trim()) return account.email.trim();
  if (account.label.trim()) return account.label.trim();
  return account.id;
};

const ampAccountLabel = (account: AmpAccountEntry): string => {
  if (account.email && account.email.trim()) return account.email.trim();
  if (account.label.trim()) return account.label.trim();
  return account.id;
};

const auggieAccountLabel = (account: AuggieAccountEntry): string => {
  if (account.email && account.email.trim()) return account.email.trim();
  if (account.label.trim()) return account.label.trim();
  return account.id;
};

export const defaultEndpointBaseUrlForProvider = (providerId: string): string => {
  if (providerId === "codex") return "https://api.openai.com/v1";
  if (providerId === "claude-crp") return "https://api.anthropic.com/v1";
  return "";
};

export const buildHarnessAuthRows = ({
  provider_id,
  selected_source_kind,
  selected_endpoint_id,
  endpoints,
  codex_accounts,
  codex_active_account_id,
  claude_accounts,
  claude_active_account_id,
  gemini_accounts,
  gemini_active_account_id,
  kimi_accounts,
  kimi_active_account_id,
  copilot_accounts,
  copilot_active_account_id,
  kiro_accounts,
  kiro_active_account_id,
  cursor_accounts,
  cursor_active_account_id,
  amp_accounts = [],
  amp_active_account_id,
  auggie_accounts = [],
  auggie_active_account_id,
}: BuildHarnessAuthRowsArgs): HarnessAuthRow[] => {
  const rows: HarnessAuthRow[] = [];

  if (provider_id === "codex" && codex_accounts.length > 0) {
    for (const account of codex_accounts) {
      rows.push({
        key: `subscription:${account.id}`,
        kind: "subscription",
        label: codexAccountLabel(account),
        active: selected_source_kind === "subscription" && codex_active_account_id === account.id,
        selectable: true,
        account_id: account.id,
        can_delete: true,
      });
    }
  }

  if (provider_id === "claude-crp" && claude_accounts.length > 0) {
    for (const account of claude_accounts) {
      rows.push({
        key: `subscription:${account.id}`,
        kind: "subscription",
        label: claudeAccountLabel(account),
        active: selected_source_kind === "subscription" && claude_active_account_id === account.id,
        selectable: true,
        account_id: account.id,
        can_delete: true,
      });
    }
  }

  if (provider_id === "gemini" && gemini_accounts.length > 0) {
    for (const account of gemini_accounts) {
      rows.push({
        key: `subscription:${account.id}`,
        kind: "subscription",
        label: geminiAccountLabel(account),
        active: selected_source_kind === "subscription" && gemini_active_account_id === account.id,
        selectable: true,
        account_id: account.id,
        can_delete: true,
      });
    }
  }

  if (provider_id === "kimi" && kimi_accounts.length > 0) {
    for (const account of kimi_accounts) {
      rows.push({
        key: `subscription:${account.id}`,
        kind: "subscription",
        label: kimiAccountLabel(account),
        active: selected_source_kind === "subscription" && kimi_active_account_id === account.id,
        selectable: true,
        account_id: account.id,
        can_delete: true,
      });
    }
  }

  if (provider_id === "copilot" && copilot_accounts.length > 0) {
    for (const account of copilot_accounts) {
      rows.push({
        key: `subscription:${account.id}`,
        kind: "subscription",
        label: copilotAccountLabel(account),
        active:
          selected_source_kind === "subscription" && copilot_active_account_id === account.id,
        selectable: true,
        account_id: account.id,
        can_delete: true,
      });
    }
  }

  if (provider_id === "kiro" && kiro_accounts.length > 0) {
    for (const account of kiro_accounts) {
      rows.push({
        key: `subscription:${account.id}`,
        kind: "subscription",
        label: kiroAccountLabel(account),
        active: selected_source_kind === "subscription" && kiro_active_account_id === account.id,
        selectable: true,
        account_id: account.id,
        can_delete: true,
      });
    }
  }

  if (provider_id === "cursor" && cursor_accounts.length > 0) {
    // Cursor is currently treated as unmanaged subscription auth in settings/workbench.
    // Keep any legacy managed account records out of the primary row model.
    void cursor_accounts;
    void cursor_active_account_id;
  }

  if (provider_id === "amp" && amp_accounts.length > 0) {
    for (const account of amp_accounts) {
      rows.push({
        key: `subscription:${account.id}`,
        kind: "subscription",
        label: ampAccountLabel(account),
        active: selected_source_kind === "subscription" && amp_active_account_id === account.id,
        selectable: true,
        account_id: account.id,
        can_delete: true,
      });
    }
  }

  if (provider_id === "auggie" && auggie_accounts.length > 0) {
    for (const account of auggie_accounts) {
      rows.push({
        key: `subscription:${account.id}`,
        kind: "subscription",
        label: auggieAccountLabel(account),
        active: selected_source_kind === "subscription" && auggie_active_account_id === account.id,
        selectable: true,
        account_id: account.id,
        can_delete: true,
      });
    }
  }

  if (provider_id === "cursor") {
    rows.push({
      key: "subscription:cursor-unmanaged",
      kind: "subscription",
      label: "Cursor host login (unmanaged)",
      detail: "Uses cursor-agent login on this target host.",
      active: selected_source_kind === "subscription",
      selectable: true,
      can_delete: false,
    });
  }

  for (const endpoint of endpoints) {
    const modelCount = Array.isArray(endpoint.model_catalog_models)
      ? endpoint.model_catalog_models.length
      : 0;
    rows.push({
      key: `endpoint:${endpoint.id}`,
      kind: "api_key",
      label: endpoint.name,
      detail: modelCount > 0 ? `${modelCount} discovered models` : undefined,
      active: selected_source_kind === "endpoint" && selected_endpoint_id === endpoint.id,
      selectable: true,
      endpoint_id: endpoint.id,
      can_delete: true,
      verification_status: endpoint.last_verification_status,
      last_error: endpoint.last_error ?? null,
      model_catalog_status: endpoint.model_catalog_status ?? "unknown",
      model_catalog_error: endpoint.model_catalog_error ?? null,
      model_count: modelCount,
      model_catalog_fetched_at: endpoint.model_catalog_fetched_at ?? null,
    });
  }

  return rows;
};
