import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { HarnessAuthModalState } from "../../SettingsPage.types";
import type { useHarnessAuthenticationController } from "../hooks/useHarnessAuthenticationController";
import { HarnessAuthenticationSection } from "./HarnessAuthenticationSection";

const mockUseHarnessAuthenticationController = vi.fn();

vi.mock("../hooks/useHarnessAuthenticationController", () => ({
  useHarnessAuthenticationController: (...args: unknown[]) => mockUseHarnessAuthenticationController(...args),
}));

type HarnessAuthController = ReturnType<typeof useHarnessAuthenticationController>;

function makeGeminiSubscriptionModal(): HarnessAuthModalState {
  return {
    provider_id: "gemini",
    stage: "subscription",
    endpoint_id: null,
    endpoint_provider_id: "gemini",
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
    subscription_auth_url: null,
    subscription_status: null,
    subscription_busy: false,
    api_key_busy: false,
  };
}

function makeCopilotSubscriptionModal(): HarnessAuthModalState {
  return {
    provider_id: "copilot",
    stage: "subscription",
    endpoint_provider_id: "openai",
    gemini_endpoint_auth_type: "gemini_api_key",
    endpoint_name: "",
    base_url: "",
    api_key: "",
    subscription_label: "",
    subscription_token: "",
    subscription_email: "",
    subscription_provider: "",
    subscription_credentials_json: "",
    subscription_config_toml: "",
    subscription_auth_token_json: "",
    subscription_oauth_creds_json: "",
    subscription_google_accounts_json: "",
    subscription_device_code: null,
    subscription_status: null,
    subscription_busy: false,
    api_key_busy: false,
  };
}

function makeController(
  overrides: Partial<HarnessAuthController> = {},
): HarnessAuthController {
  const asyncNoop = async () => {};
  return {
    providers: [],
    installs: {},
    installBusy: null,
    onInstallAll: asyncNoop,
    onInstall: asyncNoop,
    providerHarnessConfig: {},
    providerHarnessBusy: {},
    codexAccounts: null,
    codexAccountsBusy: false,
    claudeAccounts: null,
    claudeAccountsBusy: false,
    geminiAccounts: null,
    geminiAccountsBusy: false,
    qwenAccounts: null,
    qwenAccountsBusy: false,
    kimiAccounts: null,
    kimiAccountsBusy: false,
    mistralAccounts: null,
    mistralAccountsBusy: false,
    copilotAccounts: null,
    copilotAccountsBusy: false,
    kiroAccounts: null,
    kiroAccountsBusy: false,
    cursorAccounts: null,
    cursorAccountsBusy: false,
    ampAccounts: null,
    ampAccountsBusy: false,
    harnessAuthModal: makeGeminiSubscriptionModal(),
    openHarnessAuthModal: () => {},
    closeHarnessAuthModal: () => {},
    patchHarnessAuthModal: () => {},
    submitHarnessSubscriptionModal: asyncNoop,
    submitHarnessApiKeyModal: asyncNoop,
    onSelectHarnessAuthRow: asyncNoop,
    onDeleteProviderEndpoint: asyncNoop,
    onCodexDelete: asyncNoop,
    onClaudeDelete: asyncNoop,
    onGeminiDelete: asyncNoop,
    onQwenDelete: asyncNoop,
    onKimiDelete: asyncNoop,
    onMistralDelete: asyncNoop,
    onCopilotDelete: asyncNoop,
    onKiroDelete: asyncNoop,
    onCursorDelete: asyncNoop,
    onAmpDelete: asyncNoop,
    providerError: null,
    supportsHarnessEndpointConfig: () => true,
    harnessEndpointRequiresBaseUrl: () => false,
    ...overrides,
  } as HarnessAuthController;
}

describe("HarnessAuthenticationSection Gemini subscription modal", () => {
  it("shows oauth-only sign-in flow without manual JSON fields", () => {
    mockUseHarnessAuthenticationController.mockReturnValue(makeController());

    render(
      <HarnessAuthenticationSection
        workspaceId="ws-1"
        active
        modalOnly
      />,
    );

    expect(
      screen.getByText("Sign in with Google to capture managed Gemini OAuth credentials automatically."),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sign in with Google" })).toBeInTheDocument();
    expect(screen.queryByText("Leave JSON fields blank to run guided browser sign-in.")).not.toBeInTheDocument();
    expect(screen.queryByText("OAuth Credentials JSON (fallback)")).not.toBeInTheDocument();
    expect(screen.queryByText("Google Accounts JSON (optional)")).not.toBeInTheDocument();
  });

  it("submits guided sign-in when clicking the subscription action", () => {
    const submitHarnessSubscriptionModal = vi.fn(async () => {});
    mockUseHarnessAuthenticationController.mockReturnValue(
      makeController({ submitHarnessSubscriptionModal }),
    );

    render(
      <HarnessAuthenticationSection
        workspaceId="ws-1"
        active
        modalOnly
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Sign in with Google" }));
    expect(submitHarnessSubscriptionModal).toHaveBeenCalledTimes(1);
  });

  it("shows manual browser-open guidance text when auth URL is present", () => {
    const authUrl = "https://example.com/oauth/start";
    mockUseHarnessAuthenticationController.mockReturnValue(
      makeController({
        harnessAuthModal: {
          ...makeGeminiSubscriptionModal(),
          subscription_status: "Couldn't open browser automatically. Use the sign-in link below.",
          subscription_auth_url: authUrl,
        },
      }),
    );

    render(
      <HarnessAuthenticationSection
        workspaceId="ws-1"
        active
        modalOnly
      />,
    );

    expect(screen.getByText("Couldn't open browser automatically. Use the sign-in link below.")).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Open sign-in link" })).not.toBeInTheDocument();
    expect(authUrl).toContain("/oauth/");
  });
});

describe("HarnessAuthenticationSection Copilot subscription modal", () => {
  it("shows token-based Copilot subscription fields", () => {
    mockUseHarnessAuthenticationController.mockReturnValue(
      makeController({ harnessAuthModal: makeCopilotSubscriptionModal() }),
    );

    render(
      <HarnessAuthenticationSection
        workspaceId="ws-1"
        active
        modalOnly
      />,
    );

    expect(
      screen.getByText("Paste a GitHub token with Copilot entitlement for the managed Copilot account."),
    ).toBeInTheDocument();
    expect(screen.getByText("Token")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Copilot subscription")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("you@example.com")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save subscription" })).toBeInTheDocument();
  });

  it("does not render device code field even when copilot modal state carries one", () => {
    mockUseHarnessAuthenticationController.mockReturnValue(
      makeController({
        harnessAuthModal: {
          ...makeCopilotSubscriptionModal(),
          subscription_device_code: "ABCD-1234",
          subscription_status: "Waiting for GitHub sign-in to complete in your browser...",
        },
      }),
    );

    render(
      <HarnessAuthenticationSection
        workspaceId="ws-1"
        active
        modalOnly
      />,
    );

    expect(screen.queryByText("GitHub device code")).not.toBeInTheDocument();
    expect(screen.queryByDisplayValue("ABCD-1234")).not.toBeInTheDocument();
  });
});
