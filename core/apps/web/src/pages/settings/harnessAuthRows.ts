import type { CodexAccountEntry, HarnessEndpointRecord, HarnessSourceKind } from "../../api/client";

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
};

type BuildHarnessAuthRowsArgs = {
  provider_id: string;
  selected_source_kind: HarnessSourceKind;
  selected_endpoint_id?: string | null;
  endpoints: HarnessEndpointRecord[];
  codex_accounts: CodexAccountEntry[];
  codex_active_account_id?: string | null;
};

const codexAccountLabel = (account: CodexAccountEntry): string => {
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

  for (const endpoint of endpoints) {
    rows.push({
      key: `endpoint:${endpoint.id}`,
      kind: "api_key",
      label: endpoint.name,
      active: selected_source_kind === "endpoint" && selected_endpoint_id === endpoint.id,
      selectable: true,
      endpoint_id: endpoint.id,
      can_delete: true,
      verification_status: endpoint.last_verification_status,
      last_error: endpoint.last_error ?? null,
    });
  }

  return rows;
};
