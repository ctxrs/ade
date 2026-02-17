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
      endpoints: [],
    });

    expect(rows).toHaveLength(0);
  });
});
