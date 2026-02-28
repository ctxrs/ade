import { describe, expect, it } from "vitest";
import {
  extractGithubDeviceCodeFromAuthUrl,
  resolveUpsertedEndpoint,
  resolveHarnessAuthModalInitialStage,
  shouldSkipDuplicateAmpLoginStart,
  shouldAutoOpenKimiAuthUrl,
  shouldCompleteClaudeLoginWithCallbackCode,
  shouldOpenPolledAuthUrlForStatus,
  shouldOpenPolledClaudeAuthUrl,
  shouldAutoOpenCopilotAuthUrl,
  supportsHarnessSubscriptionAuth,
  takeNextClaudeAuthUrlToOpen,
  toErrorObject,
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

describe("shouldSkipDuplicateAmpLoginStart", () => {
  it("skips duplicate Amp starts while one is in flight", () => {
    expect(shouldSkipDuplicateAmpLoginStart({
      providerId: "amp",
      ampLoginInFlight: true,
    })).toBe(true);
    expect(shouldSkipDuplicateAmpLoginStart({
      providerId: "amp",
      ampLoginInFlight: false,
    })).toBe(false);
    expect(shouldSkipDuplicateAmpLoginStart({
      providerId: "gemini",
      ampLoginInFlight: true,
    })).toBe(false);
  });
});

describe("shouldOpenPolledAuthUrlForStatus", () => {
  it("opens auth url only while status is pending", () => {
    expect(shouldOpenPolledAuthUrlForStatus("pending")).toBe(true);
    expect(shouldOpenPolledAuthUrlForStatus("success")).toBe(false);
    expect(shouldOpenPolledAuthUrlForStatus("failed")).toBe(false);
    expect(shouldOpenPolledAuthUrlForStatus("timeout")).toBe(false);
  });
});

describe("shouldAutoOpenCopilotAuthUrl", () => {
  it("does not auto-open copilot auth urls from the web app", () => {
    expect(shouldAutoOpenCopilotAuthUrl("https://github.com/login/device")).toBe(false);
    expect(
      shouldAutoOpenCopilotAuthUrl("https://github.com/login/device?user_code=ABCD-1234"),
    ).toBe(false);
    expect(shouldAutoOpenCopilotAuthUrl("https://github.com/login/oauth/authorize?client_id=abc")).toBe(
      false,
    );
    expect(shouldAutoOpenCopilotAuthUrl("https://example.com/auth")).toBe(false);
  });
});

describe("extractGithubDeviceCodeFromAuthUrl", () => {
  it("extracts user_code from github device url", () => {
    expect(
      extractGithubDeviceCodeFromAuthUrl("https://github.com/login/device?user_code=ABCD-1234"),
    ).toBe("ABCD-1234");
  });

  it("returns null for non-device urls", () => {
    expect(extractGithubDeviceCodeFromAuthUrl("https://github.com/login/device")).toBeNull();
    expect(extractGithubDeviceCodeFromAuthUrl("https://example.com/login/device?user_code=ABCD-1234")).toBeNull();
    expect(extractGithubDeviceCodeFromAuthUrl("")).toBeNull();
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

describe("supportsHarnessSubscriptionAuth", () => {
  it("returns false for API-key-only providers", () => {
    expect(supportsHarnessSubscriptionAuth("opencode")).toBe(false);
    expect(supportsHarnessSubscriptionAuth("pi")).toBe(false);
    expect(supportsHarnessSubscriptionAuth("cursor")).toBe(false);
  });

  it("returns true for subscription-capable providers", () => {
    expect(supportsHarnessSubscriptionAuth("codex")).toBe(true);
    expect(supportsHarnessSubscriptionAuth("claude-crp")).toBe(true);
    expect(supportsHarnessSubscriptionAuth("gemini")).toBe(true);
  });
});

describe("resolveHarnessAuthModalInitialStage", () => {
  it("routes API-key-only providers directly to api_key", () => {
    expect(resolveHarnessAuthModalInitialStage("opencode")).toBe("api_key");
    expect(resolveHarnessAuthModalInitialStage("pi")).toBe("api_key");
    expect(resolveHarnessAuthModalInitialStage("cursor")).toBe("api_key");
  });

  it("keeps choose stage for providers supporting both methods", () => {
    expect(resolveHarnessAuthModalInitialStage("codex")).toBe("choose");
    expect(resolveHarnessAuthModalInitialStage("claude-crp")).toBe("choose");
  });
});
