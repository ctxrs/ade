import { describe, expect, it } from "vitest";
import {
  resolveUpsertedEndpoint,
  shouldAutoOpenKimiAuthUrl,
  shouldCompleteClaudeLoginWithCallbackCode,
  shouldOpenPolledClaudeAuthUrl,
  takeNextClaudeAuthUrlToOpen,
} from "./useHarnessAuthenticationController";

describe("takeNextClaudeAuthUrlToOpen", () => {
  it("normalizes and deduplicates urls", () => {
    const opened = new Set<string>();

    const first = takeNextClaudeAuthUrlToOpen(" https://claude.ai/oauth/authorize?code=abc ", opened);
    const duplicate = takeNextClaudeAuthUrlToOpen("https://claude.ai/oauth/authorize?code=abc", opened);

    expect(first).toBe("https://claude.ai/oauth/authorize?code=abc");
    expect(duplicate).toBeNull();
  });

  it("ignores empty values", () => {
    const opened = new Set<string>();

    expect(takeNextClaudeAuthUrlToOpen("", opened)).toBeNull();
    expect(takeNextClaudeAuthUrlToOpen("   ", opened)).toBeNull();
    expect(takeNextClaudeAuthUrlToOpen(null, opened)).toBeNull();
    expect(takeNextClaudeAuthUrlToOpen(undefined, opened)).toBeNull();
  });

  it("allows distinct urls once each", () => {
    const opened = new Set<string>();

    const first = takeNextClaudeAuthUrlToOpen("https://claude.ai/oauth/authorize?code=abc", opened);
    const second = takeNextClaudeAuthUrlToOpen("https://claude.ai/oauth/authorize?code=def", opened);
    const secondDuplicate = takeNextClaudeAuthUrlToOpen("https://claude.ai/oauth/authorize?code=def", opened);

    expect(first).toBe("https://claude.ai/oauth/authorize?code=abc");
    expect(second).toBe("https://claude.ai/oauth/authorize?code=def");
    expect(secondDuplicate).toBeNull();
  });
});

describe("shouldCompleteClaudeLoginWithCallbackCode", () => {
  it("returns true for pending Claude login with non-setup callback code", () => {
    expect(shouldCompleteClaudeLoginWithCallbackCode({
      providerId: "claude-crp",
      subscriptionBusy: true,
      pendingLoginId: "login-123",
      token: "ePBMdWetJlSbZ0aR#state",
    })).toBe(true);
  });

  it("returns false for setup token, missing login, or non-claude providers", () => {
    expect(shouldCompleteClaudeLoginWithCallbackCode({
      providerId: "claude-crp",
      subscriptionBusy: true,
      pendingLoginId: "login-123",
      token: "sk-ant-oat01-abc",
    })).toBe(false);
    expect(shouldCompleteClaudeLoginWithCallbackCode({
      providerId: "claude-crp",
      subscriptionBusy: true,
      pendingLoginId: null,
      token: "ePBMdWetJlSbZ0aR#state",
    })).toBe(false);
    expect(shouldCompleteClaudeLoginWithCallbackCode({
      providerId: "codex",
      subscriptionBusy: true,
      pendingLoginId: "login-123",
      token: "ePBMdWetJlSbZ0aR#state",
    })).toBe(false);
  });
});

describe("resolveUpsertedEndpoint", () => {
  it("reuses the requested endpoint id during retry", () => {
    const endpoint = resolveUpsertedEndpoint({
      requestedEndpointId: "ep-2",
      previousEndpointIds: new Set(["ep-1", "ep-2"]),
      nextEndpoints: [
        {
          id: "ep-1",
          provider_id: "codex",
          name: "Primary",
          base_url: "https://api.example.com/v1",
          api_shape: "openai_responses",
          auth_type: "bearer",
          model_override: null,
          created_at: "2026-02-20T00:00:00Z",
          updated_at: "2026-02-20T00:00:00Z",
          last_verification_status: "unknown",
          last_verification_at: null,
          last_error: null,
          has_api_key: true,
        },
        {
          id: "ep-2",
          provider_id: "codex",
          name: "Primary",
          base_url: "https://api.example.com/v1",
          api_shape: "openai_responses",
          auth_type: "bearer",
          model_override: null,
          created_at: "2026-02-20T00:00:00Z",
          updated_at: "2026-02-20T00:00:00Z",
          last_verification_status: "unknown",
          last_verification_at: null,
          last_error: null,
          has_api_key: true,
        },
      ],
      name: "Primary",
      normalizedBase: "https://api.example.com/v1",
      geminiAuthType: null,
    });
    expect(endpoint?.id).toBe("ep-2");
  });

  it("selects the newly created endpoint when creating for the first time", () => {
    const endpoint = resolveUpsertedEndpoint({
      requestedEndpointId: null,
      previousEndpointIds: new Set(["ep-1"]),
      nextEndpoints: [
        {
          id: "ep-1",
          provider_id: "codex",
          name: "Existing",
          base_url: "https://api.example.com/v1",
          api_shape: "openai_responses",
          auth_type: "bearer",
          model_override: null,
          created_at: "2026-02-20T00:00:00Z",
          updated_at: "2026-02-20T00:00:00Z",
          last_verification_status: "unknown",
          last_verification_at: null,
          last_error: null,
          has_api_key: true,
        },
        {
          id: "ep-2",
          provider_id: "codex",
          name: "OpenRouter",
          base_url: "https://openrouter.ai/api/v1",
          api_shape: "openai_responses",
          auth_type: "bearer",
          model_override: null,
          created_at: "2026-02-20T00:00:00Z",
          updated_at: "2026-02-20T00:00:00Z",
          last_verification_status: "unknown",
          last_verification_at: null,
          last_error: null,
          has_api_key: true,
        },
      ],
      name: "OpenRouter",
      normalizedBase: "https://openrouter.ai/api/v1",
      geminiAuthType: null,
    });
    expect(endpoint?.id).toBe("ep-2");
  });
});

describe("shouldOpenPolledClaudeAuthUrl", () => {
  it("waits for grace window before opening when no initial url exists", () => {
    expect(shouldOpenPolledClaudeAuthUrl({
      loginStartedAtMs: 1_000,
      initialAuthUrl: null,
      polledAuthUrl: "https://claude.ai/oauth/authorize?code=abc",
      nowMs: 5_500,
    })).toBe(false);
    expect(shouldOpenPolledClaudeAuthUrl({
      loginStartedAtMs: 1_000,
      initialAuthUrl: null,
      polledAuthUrl: "https://claude.ai/oauth/authorize?code=abc",
      nowMs: 6_000,
    })).toBe(true);
  });

  it("opens upgraded polled url immediately when initial url differs", () => {
    expect(shouldOpenPolledClaudeAuthUrl({
      loginStartedAtMs: 1_000,
      initialAuthUrl: "https://claude.ai/oauth/authorize?code=short",
      polledAuthUrl: "https://claude.ai/oauth/authorize?code=complete",
      nowMs: 1_001,
    })).toBe(true);
  });

  it("does not reopen same initial url", () => {
    expect(shouldOpenPolledClaudeAuthUrl({
      loginStartedAtMs: 1_000,
      initialAuthUrl: "https://claude.ai/oauth/authorize?code=same",
      polledAuthUrl: "https://claude.ai/oauth/authorize?code=same",
      nowMs: 9_999,
    })).toBe(false);
  });
});

describe("shouldAutoOpenKimiAuthUrl", () => {
  it("keeps Kimi browser open behavior single-source to avoid duplicate tabs", () => {
    expect(shouldAutoOpenKimiAuthUrl()).toBe(false);
  });
});
