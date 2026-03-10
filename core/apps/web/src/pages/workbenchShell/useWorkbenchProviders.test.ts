import { describe, expect, it } from "vitest";
import type { ProviderOptions } from "../../api/client";
import { resolveProviderOptionsUpdate, shouldHydrateProviderModels } from "./useWorkbenchProviders";

const baseOptions = (providerId: string): ProviderOptions => ({
  provider_id: providerId,
  workspace_id: "ws-test",
  supports_load: false,
  auth_required: false,
  has_active_auth: true,
  auth_mode: "subscription",
  source: {
    provider_id: providerId,
    selected_source_kind: "subscription",
    selected_endpoint_id: null,
    endpoints: [],
  },
  probed_at: new Date().toISOString(),
});

describe("shouldHydrateProviderModels", () => {
  it("requests hydration for claude subscription auth when models are missing", () => {
    expect(shouldHydrateProviderModels("claude-crp", baseOptions("claude-crp"))).toBe(true);
  });

  it("requests hydration for endpoint-selected sources when models are missing", () => {
    const options: ProviderOptions = {
      ...baseOptions("claude-crp"),
      source: {
        provider_id: "claude-crp",
        selected_source_kind: "endpoint",
        selected_endpoint_id: "ep-1",
        endpoints: [],
      },
    };
    expect(shouldHydrateProviderModels("claude-crp", options)).toBe(true);
  });

  it("does not request hydration when models already exist", () => {
    const options: ProviderOptions = {
      ...baseOptions("claude-crp"),
      models: {
        models: [{ id: "anthropic/claude-sonnet-4.5" }],
        current_model_id: "anthropic/claude-sonnet-4.5",
      },
    };
    expect(shouldHydrateProviderModels("claude-crp", options)).toBe(false);
  });

  it("does not request hydration for providers without CRP model discovery", () => {
    expect(shouldHydrateProviderModels("gemini", baseOptions("gemini"))).toBe(false);
  });

  it("does not keep passively hydrating after a failed probe", () => {
    const options: ProviderOptions = {
      ...baseOptions("codex"),
      probe_ok: false,
      probe_error: "crp runtime closed before models.list response",
    };
    expect(shouldHydrateProviderModels("codex", options)).toBe(false);
  });

  it("allows an explicit retry after a failed probe", () => {
    const options: ProviderOptions = {
      ...baseOptions("codex"),
      probe_ok: false,
      probe_error: "crp runtime closed before models.list response",
    };
    expect(shouldHydrateProviderModels("codex", options, "explicit")).toBe(true);
  });
});

describe("resolveProviderOptionsUpdate", () => {
  it("preserves last known models when a later payload drops them for the same source", () => {
    const previous: ProviderOptions = {
      ...baseOptions("codex"),
      models: {
        models: [{ id: "gpt-5" }],
        current_model_id: "gpt-5",
      },
    };
    const next: ProviderOptions = {
      ...baseOptions("codex"),
      probed_at: "2026-03-09T00:00:05.000Z",
      probe_ok: false,
      probe_error: "crp runtime closed before models.list response",
    };

    expect(resolveProviderOptionsUpdate(previous, next)).toEqual({
      ...next,
      models: previous.models,
    });
  });

  it("keeps a failed probe sticky across bootstrap summaries until a real retry", () => {
    const previous: ProviderOptions = {
      ...baseOptions("codex"),
      probe_ok: false,
      probe_error: "crp runtime closed before models.list response",
    };
    const next: ProviderOptions = {
      ...baseOptions("codex"),
      probed_at: "2026-03-09T00:00:05.000Z",
    };

    expect(resolveProviderOptionsUpdate(previous, next)).toEqual({
      ...next,
      probe_ok: false,
      probe_error: previous.probe_error,
    });
  });
});
