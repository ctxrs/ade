import { describe, expect, it } from "vitest";

import { shouldShowLoadingProviderModels } from "./WorkbenchComposer.utils";

describe("shouldShowLoadingProviderModels", () => {
  it("keeps subscription-backed discovery providers in a loading state until models arrive", () => {
    expect(shouldShowLoadingProviderModels("codex", {
      provider_id: "codex",
      workspace_id: "ws-test",
      supports_load: false,
      auth_required: false,
      has_active_auth: true,
      auth_mode: "subscription",
      source: {
        provider_id: "codex",
        selected_source_kind: "subscription",
        selected_endpoint_id: null,
        endpoints: [],
      },
      probed_at: "2026-03-10T00:00:00.000Z",
    })).toBe(true);
  });

  it("does not show loading for endpoint-backed providers without a discovered catalog", () => {
    expect(shouldShowLoadingProviderModels("codex", {
      provider_id: "codex",
      workspace_id: "ws-test",
      supports_load: false,
      auth_required: false,
      has_active_auth: true,
      auth_mode: "endpoint",
      source: {
        provider_id: "codex",
        selected_source_kind: "endpoint",
        selected_endpoint_id: "ep-1",
        endpoints: [
          {
            id: "ep-1",
            provider_id: "codex",
            name: "Primary",
            base_url: "https://api.example.com/v1",
            api_shape: "openai_responses",
            auth_type: "bearer",
            model_override: null,
            created_at: "2026-03-10T00:00:00.000Z",
            updated_at: "2026-03-10T00:00:00.000Z",
            last_verification_status: "valid",
            last_verification_at: "2026-03-10T00:00:00.000Z",
            last_error: null,
            has_api_key: true,
            model_catalog_status: "manual_only",
            model_catalog_fetched_at: null,
            model_catalog_error: null,
            model_catalog_models: [],
            manual_model_ids: [],
            model_catalog_source: null,
          },
        ],
      },
      probed_at: "2026-03-10T00:00:00.000Z",
    })).toBe(false);
  });

  it("stops loading once the provider options already include models", () => {
    expect(shouldShowLoadingProviderModels("claude-crp", {
      provider_id: "claude-crp",
      workspace_id: "ws-test",
      supports_load: false,
      auth_required: false,
      has_active_auth: true,
      auth_mode: "subscription",
      source: {
        provider_id: "claude-crp",
        selected_source_kind: "subscription",
        selected_endpoint_id: null,
        endpoints: [],
      },
      models: {
        models: [{ id: "default/low" }, { id: "default/medium" }],
        current_model_id: "default/medium",
      },
      probed_at: "2026-03-10T00:00:00.000Z",
    })).toBe(false);
  });
});
