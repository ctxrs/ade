import { describe, expect, it } from "vitest";
import type { ProviderOptions } from "../../api/client";
import { shouldHydrateProviderModels } from "./useWorkbenchProviders";

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

  it("does not request hydration for endpoint-selected sources", () => {
    const options: ProviderOptions = {
      ...baseOptions("claude-crp"),
      source: {
        provider_id: "claude-crp",
        selected_source_kind: "endpoint",
        selected_endpoint_id: "ep-1",
        endpoints: [],
      },
    };
    expect(shouldHydrateProviderModels("claude-crp", options)).toBe(false);
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
});

