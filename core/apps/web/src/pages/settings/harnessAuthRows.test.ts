import { describe, expect, it } from "vitest";
import { buildHarnessAuthRows, defaultEndpointBaseUrlForProvider } from "./harnessAuthRows";

describe("defaultEndpointBaseUrlForProvider", () => {
  it("returns provider-specific defaults", () => {
    expect(defaultEndpointBaseUrlForProvider("codex")).toBe("https://api.openai.com/v1");
    expect(defaultEndpointBaseUrlForProvider("claude-crp")).toBe("https://api.anthropic.com/v1");
    expect(defaultEndpointBaseUrlForProvider("gemini")).toBe("");
  });
});

describe("buildHarnessAuthRows", () => {
  it("builds codex subscription rows per account and marks active", () => {
    const rows = buildHarnessAuthRows({
      provider_id: "codex",
      selected_source_kind: "subscription",
      selected_endpoint_id: null,
      endpoints: [],
      codex_active_account_id: "b",
      claude_accounts: [],
      claude_active_account_id: null,
      gemini_accounts: [],
      gemini_active_account_id: null,
      kimi_accounts: [],
      kimi_active_account_id: null,
      copilot_accounts: [],
      copilot_active_account_id: null,
      kiro_accounts: [],
      kiro_active_account_id: null,
      cursor_accounts: [],
      cursor_active_account_id: null,
      codex_accounts: [
        {
          id: "a",
          label: "Primary",
          email: "first@example.com",
          created_at: "2026-01-01T00:00:00Z",
        },
        {
          id: "b",
          label: "Backup",
          email: "second@example.com",
          created_at: "2026-01-01T00:00:00Z",
        },
      ],
    });

    const active = rows.filter((row) => row.active);
    expect(rows).toHaveLength(2);
    expect(active).toHaveLength(1);
    expect(active[0]?.account_id).toBe("b");
    expect(rows[0]?.label).toBe("first@example.com");
    expect(rows.every((row) => row.detail === undefined)).toBe(true);
    expect(rows.every((row) => row.can_delete === true)).toBe(true);
  });

  it("includes endpoint rows with active endpoint and status", () => {
    const rows = buildHarnessAuthRows({
      provider_id: "claude-crp",
      selected_source_kind: "endpoint",
      selected_endpoint_id: "ep-2",
      codex_accounts: [],
      codex_active_account_id: null,
      claude_accounts: [],
      claude_active_account_id: null,
      gemini_accounts: [],
      gemini_active_account_id: null,
      kimi_accounts: [],
      kimi_active_account_id: null,
      copilot_accounts: [],
      copilot_active_account_id: null,
      kiro_accounts: [],
      kiro_active_account_id: null,
      cursor_accounts: [],
      cursor_active_account_id: null,
      endpoints: [
        {
          id: "ep-1",
          provider_id: "claude-crp",
          name: "Key 1",
          base_url: "https://api.anthropic.com/v1",
          api_shape: "anthropic_messages",
          auth_type: "api_key",
          created_at: "2026-01-01T00:00:00Z",
          updated_at: "2026-01-01T00:00:00Z",
          has_api_key: true,
          last_verification_status: "unknown",
          last_verification_at: null,
          last_error: null,
          model_override: null,
        },
        {
          id: "ep-2",
          provider_id: "claude-crp",
          name: "Key 2",
          base_url: "https://openrouter.ai/api/v1",
          api_shape: "anthropic_messages",
          auth_type: "api_key",
          created_at: "2026-01-01T00:00:00Z",
          updated_at: "2026-01-01T00:00:00Z",
          has_api_key: true,
          last_verification_status: "valid",
          last_verification_at: null,
          last_error: null,
          model_override: null,
        },
      ],
    });

    const endpointRows = rows.filter((row) => row.kind === "api_key");
    const activeEndpoint = endpointRows.find((row) => row.active);
    expect(endpointRows).toHaveLength(2);
    expect(activeEndpoint?.endpoint_id).toBe("ep-2");
    expect(activeEndpoint?.verification_status).toBe("valid");
    expect(endpointRows.every((row) => row.detail === undefined)).toBe(true);
  });

  it("does not add placeholder subscription rows without managed entries", () => {
    const rows = buildHarnessAuthRows({
      provider_id: "claude-crp",
      selected_source_kind: "subscription",
      selected_endpoint_id: null,
      codex_accounts: [],
      codex_active_account_id: null,
      claude_accounts: [],
      claude_active_account_id: null,
      gemini_accounts: [],
      gemini_active_account_id: null,
      kimi_accounts: [],
      kimi_active_account_id: null,
      copilot_accounts: [],
      copilot_active_account_id: null,
      kiro_accounts: [],
      kiro_active_account_id: null,
      cursor_accounts: [],
      cursor_active_account_id: null,
      endpoints: [],
    });

    expect(rows).toHaveLength(0);
  });

  it("builds claude subscription rows per account and marks active", () => {
    const rows = buildHarnessAuthRows({
      provider_id: "claude-crp",
      selected_source_kind: "subscription",
      selected_endpoint_id: null,
      endpoints: [],
      codex_accounts: [],
      codex_active_account_id: null,
      claude_active_account_id: "claude-b",
      gemini_accounts: [],
      gemini_active_account_id: null,
      kimi_accounts: [],
      kimi_active_account_id: null,
      copilot_accounts: [],
      copilot_active_account_id: null,
      kiro_accounts: [],
      kiro_active_account_id: null,
      cursor_accounts: [],
      cursor_active_account_id: null,
      claude_accounts: [
        {
          id: "claude-a",
          label: "Claude Primary",
          email: "claude-a@example.com",
          created_at: "2026-01-01T00:00:00Z",
        },
        {
          id: "claude-b",
          label: "Claude Backup",
          email: "claude-b@example.com",
          created_at: "2026-01-01T00:00:00Z",
        },
      ],
    });

    expect(rows).toHaveLength(2);
    const active = rows.filter((row) => row.active);
    expect(active).toHaveLength(1);
    expect(active[0]?.account_id).toBe("claude-b");
    expect(rows[0]?.label).toBe("claude-a@example.com");
  });

  it("builds gemini subscription rows and marks active", () => {
    const rows = buildHarnessAuthRows({
      provider_id: "gemini",
      selected_source_kind: "subscription",
      selected_endpoint_id: null,
      endpoints: [],
      codex_accounts: [],
      codex_active_account_id: null,
      claude_accounts: [],
      claude_active_account_id: null,
      kimi_accounts: [],
      kimi_active_account_id: null,
      copilot_accounts: [],
      copilot_active_account_id: null,
      kiro_accounts: [],
      kiro_active_account_id: null,
      cursor_accounts: [],
      cursor_active_account_id: null,
      gemini_active_account_id: "gemini-2",
      gemini_accounts: [
        {
          id: "gemini-1",
          label: "Gemini Primary",
          email: "gemini-a@example.com",
          created_at: "2026-01-01T00:00:00Z",
        },
        {
          id: "gemini-2",
          label: "Gemini Backup",
          email: "gemini-b@example.com",
          created_at: "2026-01-01T00:00:00Z",
        },
      ],
    });

    expect(rows).toHaveLength(2);
    const active = rows.filter((row) => row.active);
    expect(active).toHaveLength(1);
    expect(active[0]?.account_id).toBe("gemini-2");
    expect(rows[0]?.label).toBe("gemini-a@example.com");
  });

  it("builds kimi/copilot/kiro subscription rows and marks active", () => {
    const kimiRows = buildHarnessAuthRows({
      provider_id: "kimi",
      selected_source_kind: "subscription",
      selected_endpoint_id: null,
      endpoints: [],
      codex_accounts: [],
      codex_active_account_id: null,
      claude_accounts: [],
      claude_active_account_id: null,
      gemini_accounts: [],
      gemini_active_account_id: null,
      kimi_active_account_id: "kimi-2",
      kimi_accounts: [
        { id: "kimi-1", label: "Kimi A", email: "kimi-a@example.com", created_at: "2026-01-01T00:00:00Z" },
        { id: "kimi-2", label: "Kimi B", email: "kimi-b@example.com", created_at: "2026-01-01T00:00:00Z" },
      ],
      copilot_accounts: [],
      copilot_active_account_id: null,
      kiro_accounts: [],
      kiro_active_account_id: null,
      cursor_accounts: [],
      cursor_active_account_id: null,
    });
    expect(kimiRows.find((row) => row.active)?.account_id).toBe("kimi-2");

    const copilotRows = buildHarnessAuthRows({
      provider_id: "copilot",
      selected_source_kind: "subscription",
      selected_endpoint_id: null,
      endpoints: [],
      codex_accounts: [],
      codex_active_account_id: null,
      claude_accounts: [],
      claude_active_account_id: null,
      gemini_accounts: [],
      gemini_active_account_id: null,
      kimi_accounts: [],
      kimi_active_account_id: null,
      copilot_active_account_id: "copilot-1",
      copilot_accounts: [
        { id: "copilot-1", label: "Copilot", email: "copilot@example.com", created_at: "2026-01-01T00:00:00Z" },
      ],
      kiro_accounts: [],
      kiro_active_account_id: null,
      cursor_accounts: [],
      cursor_active_account_id: null,
    });
    expect(copilotRows.find((row) => row.active)?.account_id).toBe("copilot-1");

    const kiroRows = buildHarnessAuthRows({
      provider_id: "kiro",
      selected_source_kind: "subscription",
      selected_endpoint_id: null,
      endpoints: [],
      codex_accounts: [],
      codex_active_account_id: null,
      claude_accounts: [],
      claude_active_account_id: null,
      gemini_accounts: [],
      gemini_active_account_id: null,
      kimi_accounts: [],
      kimi_active_account_id: null,
      copilot_accounts: [],
      copilot_active_account_id: null,
      kiro_active_account_id: "kiro-1",
      kiro_accounts: [
        { id: "kiro-1", label: "Kiro", email: "kiro@example.com", created_at: "2026-01-01T00:00:00Z" },
      ],
      cursor_accounts: [],
      cursor_active_account_id: null,
    });
    expect(kiroRows.find((row) => row.active)?.account_id).toBe("kiro-1");
  });

  it("builds cursor subscription rows and marks active without source-kind gating", () => {
    const rows = buildHarnessAuthRows({
      provider_id: "cursor",
      selected_source_kind: "endpoint",
      selected_endpoint_id: null,
      endpoints: [],
      codex_accounts: [],
      codex_active_account_id: null,
      claude_accounts: [],
      claude_active_account_id: null,
      gemini_accounts: [],
      gemini_active_account_id: null,
      kimi_accounts: [],
      kimi_active_account_id: null,
      copilot_accounts: [],
      copilot_active_account_id: null,
      kiro_accounts: [],
      kiro_active_account_id: null,
      cursor_active_account_id: "cursor-2",
      cursor_accounts: [
        { id: "cursor-1", label: "Cursor A", email: "cursor-a@example.com", created_at: "2026-01-01T00:00:00Z" },
        { id: "cursor-2", label: "Cursor B", email: "cursor-b@example.com", created_at: "2026-01-01T00:00:00Z" },
      ],
    });

    expect(rows.find((row) => row.active)?.account_id).toBe("cursor-2");
    expect(rows[0]?.label).toBe("cursor-a@example.com");
  });
});
