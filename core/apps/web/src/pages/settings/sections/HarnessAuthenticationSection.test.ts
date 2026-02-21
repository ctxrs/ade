import { describe, expect, it } from "vitest";
import type { HarnessAuthModalState } from "../../SettingsPage.types";
import {
  canSubmitSubscriptionModal,
  shouldAutoStartSubscriptionFlow,
  shouldSubmitClaudeFallbackOnEnter,
  subscriptionPrimaryActionLabel,
} from "./HarnessAuthenticationSection";

function baseModal(overrides: Partial<HarnessAuthModalState> = {}): HarnessAuthModalState {
  return {
    provider_id: "claude-crp",
    stage: "subscription",
    endpoint_id: null,
    endpoint_provider_id: "openai",
    gemini_endpoint_auth_type: "gemini_api_key",
    endpoint_name: "",
    base_url: "",
    api_key: "",
    manual_model_ids: "",
    subscription_label: "",
    subscription_token: "",
    subscription_email: "",
    subscription_provider: "",
    subscription_credentials_json: "",
    subscription_config_toml: "",
    subscription_auth_token_json: "",
    subscription_oauth_creds_json: "",
    subscription_google_accounts_json: "",
    subscription_status: null,
    subscription_busy: false,
    api_key_busy: false,
    ...overrides,
  };
}

describe("HarnessAuthenticationSection Claude fallback submit", () => {
  it("keeps submit disabled while browser flow is busy with no token", () => {
    const modal = baseModal({ subscription_busy: true });

    expect(canSubmitSubscriptionModal(modal)).toBe(false);
    expect(subscriptionPrimaryActionLabel(modal)).toBe("Waiting...");
  });

  it("allows submit while busy when Claude setup token is provided", () => {
    const modal = baseModal({
      subscription_busy: true,
      subscription_token: "sk-ant-oat01-token",
    });

    expect(canSubmitSubscriptionModal(modal)).toBe(true);
    expect(subscriptionPrimaryActionLabel(modal)).toBe("Save subscription");
    expect(shouldSubmitClaudeFallbackOnEnter(modal, "Enter")).toBe(true);
  });

  it("shows callback submit label while busy when Claude callback code is provided", () => {
    const modal = baseModal({
      subscription_busy: true,
      subscription_token: "ePBMdWetJlSbZ0aR#state",
    });

    expect(canSubmitSubscriptionModal(modal)).toBe(true);
    expect(subscriptionPrimaryActionLabel(modal)).toBe("Submit code");
  });

  it("does not submit Claude fallback for non-enter keys", () => {
    const modal = baseModal({
      subscription_busy: true,
      subscription_token: "sk-ant-oat01-token",
    });

    expect(shouldSubmitClaudeFallbackOnEnter(modal, "Tab")).toBe(false);
  });

  it("keeps codex subscription action as start sign-in when idle", () => {
    const modal = baseModal({
      provider_id: "codex",
      subscription_busy: false,
      subscription_token: "",
    });

    expect(subscriptionPrimaryActionLabel(modal)).toBe("Start sign-in");
  });

  it("shows Kimi subscription action as sign in with Kimi when idle", () => {
    const modal = baseModal({
      provider_id: "kimi",
      subscription_busy: false,
      subscription_token: "",
    });

    expect(subscriptionPrimaryActionLabel(modal)).toBe("Sign in with Kimi");
  });

  it("auto-starts browser sign-in from stage 1 for codex, claude, gemini, and kimi", () => {
    expect(shouldAutoStartSubscriptionFlow("codex")).toBe(true);
    expect(shouldAutoStartSubscriptionFlow("claude-crp")).toBe(true);
    expect(shouldAutoStartSubscriptionFlow("gemini")).toBe(true);
    expect(shouldAutoStartSubscriptionFlow("kimi")).toBe(true);
  });
});
