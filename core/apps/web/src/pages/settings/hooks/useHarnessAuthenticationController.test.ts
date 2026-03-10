import { act, render, waitFor } from "@testing-library/react";
import { createElement, useEffect } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  AmpAccountsResponse,
  HarnessProviderSourceConfig,
  ProvidersBootstrapResponse,
  ProviderAuthCheck,
} from "../../../api/client";
import {
  deleteAmpAccount,
  getCodexLogin,
  getGeminiLogin,
  listProviders,
  selectProviderHarnessSource,
  setAmpActiveAccount,
  startAmpLogin,
  startCodexLogin,
  startGeminiLogin,
  upsertProviderHarnessEndpoint,
  verifyProviderForWorkspace,
} from "../../../api/client";
import {
  invalidateProvidersBootstrap,
  loadProvidersBootstrap,
  refreshProvidersBootstrap,
} from "../../../state/providersBootstrapStore";
import {
  CLAUDE_LOGIN_COMPLETION_TIMEOUT_MS,
  CLAUDE_LOGIN_POLL_ATTEMPTS,
  CLAUDE_LOGIN_POLL_INTERVAL_MS,
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
  useHarnessAuthenticationController,
} from "./useHarnessAuthenticationController";
import type { HarnessAuthRow } from "../harnessAuthRows";
import { openExternalLink } from "../../../utils/desktop";

vi.mock("../../../api/client", async (importOriginal) => {
  const original = await importOriginal<typeof import("../../../api/client")>();
  return {
    ...original,
    deleteAmpAccount: vi.fn(),
    getCodexLogin: vi.fn(),
    getGeminiLogin: vi.fn(),
    listProviders: vi.fn(),
    selectProviderHarnessSource: vi.fn(),
    setAmpActiveAccount: vi.fn(),
    startAmpLogin: vi.fn(),
    startCodexLogin: vi.fn(),
    startGeminiLogin: vi.fn(),
    upsertProviderHarnessEndpoint: vi.fn(),
    verifyProviderForWorkspace: vi.fn(),
  };
});

vi.mock("../../../state/providersBootstrapStore", async (importOriginal) => {
  const original = await importOriginal<typeof import("../../../state/providersBootstrapStore")>();
  return {
    ...original,
    invalidateProvidersBootstrap: vi.fn(),
    loadProvidersBootstrap: vi.fn(),
    refreshProvidersBootstrap: vi.fn(),
  };
});

vi.mock("../../../state/providerInstallProgressStore", async (importOriginal) => {
  const original = await importOriginal<typeof import("../../../state/providerInstallProgressStore")>();
  return {
    ...original,
    getProviderInstallProgressSnapshot: vi.fn(() => ({})),
    subscribeProviderInstallProgress: vi.fn(() => () => {}),
    upsertProviderInstallProgress: vi.fn(),
  };
});

vi.mock("../../../utils/desktop", async (importOriginal) => {
  const original = await importOriginal<typeof import("../../../utils/desktop")>();
  return {
    ...original,
    desktopStartCodexLoginRelay: vi.fn(),
    isDesktopApp: vi.fn(() => false),
    openExternalLink: vi.fn(),
  };
});

type Controller = ReturnType<typeof useHarnessAuthenticationController>;

const requireController = (controller: Controller | null): Controller => {
  if (!controller) throw new Error("controller not ready");
  return controller;
};

type Deferred<T> = {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (error: unknown) => void;
};

const deferred = <T,>(): Deferred<T> => {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
};

const baseEndpoint = {
  id: "ep-old",
  provider_id: "codex",
  name: "Primary",
  base_url: "https://api.example.com/v1",
  api_shape: "openai_responses" as const,
  auth_type: "bearer",
  model_override: null,
  created_at: "2026-03-01T00:00:00Z",
  updated_at: "2026-03-01T00:00:00Z",
  last_verification_status: "unknown" as const,
  last_verification_at: null,
  last_error: null,
  has_api_key: true,
};

const baseCodexConfig: HarnessProviderSourceConfig = {
  provider_id: "codex",
  selected_source_kind: "subscription",
  selected_endpoint_id: null,
  endpoints: [baseEndpoint],
};

const baseAmpAccounts: AmpAccountsResponse = {
  active_account_id: "amp-1",
  accounts: [
    {
      id: "amp-1",
      label: "Amp One",
      created_at: "2026-03-01T00:00:00Z",
    },
    {
      id: "amp-2",
      label: "Amp Two",
      created_at: "2026-03-01T00:00:00Z",
    },
  ],
};

const makeBootstrap = (overrides?: Partial<ProvidersBootstrapResponse>): ProvidersBootstrapResponse => ({
  providers: [],
  provider_options: {},
  provider_harness_config: {
    codex: baseCodexConfig,
  },
  codex_accounts: {
    active_account_id: null,
    accounts: [],
    logins: [],
  },
  claude_accounts: {
    active_account_id: null,
    accounts: [],
  },
  gemini_accounts: {
    active_account_id: null,
    accounts: [],
  },
  qwen_accounts: {
    active_account_id: null,
    accounts: [],
  },
  kimi_accounts: {
    active_account_id: null,
    accounts: [],
  },
  mistral_accounts: {
    active_account_id: null,
    accounts: [],
  },
  copilot_accounts: {
    active_account_id: null,
    accounts: [],
  },
  cursor_accounts: {
    active_account_id: null,
    accounts: [],
  },
  amp_accounts: baseAmpAccounts,
  ...overrides,
});

function ControllerHarness({ onChange }: { onChange: (controller: Controller) => void }) {
  const controller = useHarnessAuthenticationController({
    workspaceId: "ws-test",
    enabled: true,
  });

  useEffect(() => {
    onChange(controller);
  }, [controller, onChange]);

  return null;
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(deleteAmpAccount).mockReset();
  vi.mocked(getCodexLogin).mockReset();
  vi.mocked(getGeminiLogin).mockReset();
  vi.mocked(listProviders).mockReset();
  vi.mocked(selectProviderHarnessSource).mockReset();
  vi.mocked(setAmpActiveAccount).mockReset();
  vi.mocked(startAmpLogin).mockReset();
  vi.mocked(startCodexLogin).mockReset();
  vi.mocked(startGeminiLogin).mockReset();
  vi.mocked(upsertProviderHarnessEndpoint).mockReset();
  vi.mocked(verifyProviderForWorkspace).mockReset();
  vi.mocked(invalidateProvidersBootstrap).mockReset();
  vi.mocked(loadProvidersBootstrap).mockReset();
  vi.mocked(refreshProvidersBootstrap).mockReset();
  vi.mocked(openExternalLink).mockReset();
  vi.mocked(listProviders).mockResolvedValue([]);
  vi.mocked(loadProvidersBootstrap).mockResolvedValue(makeBootstrap());
  vi.mocked(refreshProvidersBootstrap).mockResolvedValue(makeBootstrap());
});

describe("Claude polling duration", () => {
  it("matches the 15-minute backend login completion window", () => {
    expect(CLAUDE_LOGIN_POLL_ATTEMPTS).toBeGreaterThan(90);
    expect(CLAUDE_LOGIN_POLL_ATTEMPTS * CLAUDE_LOGIN_POLL_INTERVAL_MS).toBeGreaterThanOrEqual(
      CLAUDE_LOGIN_COMPLETION_TIMEOUT_MS,
    );
    expect(CLAUDE_LOGIN_POLL_ATTEMPTS * CLAUDE_LOGIN_POLL_INTERVAL_MS).toBeLessThan(
      CLAUDE_LOGIN_COMPLETION_TIMEOUT_MS + CLAUDE_LOGIN_POLL_INTERVAL_MS,
    );
  });
});

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
  });

  it("returns true for subscription-capable providers", () => {
    expect(supportsHarnessSubscriptionAuth("codex")).toBe(true);
    expect(supportsHarnessSubscriptionAuth("claude-crp")).toBe(true);
    expect(supportsHarnessSubscriptionAuth("gemini")).toBe(true);
    expect(supportsHarnessSubscriptionAuth("cursor")).toBe(true);
  });
});

describe("resolveHarnessAuthModalInitialStage", () => {
  it("routes API-key-only providers directly to api_key", () => {
    expect(resolveHarnessAuthModalInitialStage("opencode")).toBe("api_key");
    expect(resolveHarnessAuthModalInitialStage("pi")).toBe("api_key");
  });

  it("keeps choose stage for providers supporting both methods", () => {
    expect(resolveHarnessAuthModalInitialStage("codex")).toBe("choose");
    expect(resolveHarnessAuthModalInitialStage("claude-crp")).toBe("choose");
    expect(resolveHarnessAuthModalInitialStage("cursor")).toBe("choose");
  });
});

describe("useHarnessAuthenticationController", () => {
  it("restores the previous provider source when endpoint verification fails", async () => {
    let controller: Controller | null = null;
    const freshEndpoint = {
      ...baseEndpoint,
      id: "ep-new",
      name: "Secondary",
      updated_at: "2026-03-02T00:00:00Z",
    };
    const afterUpsert: HarnessProviderSourceConfig = {
      ...baseCodexConfig,
      endpoints: [...baseCodexConfig.endpoints, freshEndpoint],
    };
    const selectedEndpointConfig: HarnessProviderSourceConfig = {
      ...afterUpsert,
      selected_source_kind: "endpoint",
      selected_endpoint_id: freshEndpoint.id,
    };
    const verifyFailure: ProviderAuthCheck = {
      provider_id: "codex",
      workspace_id: "ws-test",
      status: "failed",
      message: "bad endpoint",
    };

    vi.mocked(upsertProviderHarnessEndpoint).mockResolvedValue(afterUpsert);
    vi.mocked(selectProviderHarnessSource)
      .mockResolvedValueOnce(selectedEndpointConfig)
      .mockResolvedValueOnce(baseCodexConfig);
    vi.mocked(refreshProvidersBootstrap)
      .mockResolvedValueOnce(makeBootstrap({
        provider_harness_config: {
          codex: selectedEndpointConfig,
        },
      }))
      .mockResolvedValueOnce(makeBootstrap());
    vi.mocked(verifyProviderForWorkspace).mockResolvedValue(verifyFailure);

    render(createElement(ControllerHarness, {
      onChange: (next) => {
        controller = next;
      },
    }));

    await waitFor(() => {
      expect(controller).not.toBeNull();
      expect(vi.mocked(loadProvidersBootstrap)).toHaveBeenCalledWith("ws-test");
    });

    await act(async () => {
      controller?.openHarnessAuthModal("codex");
    });
    await act(async () => {
      controller?.patchHarnessAuthModal({
        stage: "api_key",
        api_key: "sk-test",
      });
    });
    await act(async () => {
      await controller?.submitHarnessApiKeyModal();
    });

    await waitFor(() => {
      expect(vi.mocked(selectProviderHarnessSource)).toHaveBeenNthCalledWith(1, "codex", "endpoint", "ep-new");
      expect(vi.mocked(selectProviderHarnessSource)).toHaveBeenNthCalledWith(2, "codex", "subscription", null);
      expect(controller?.providerError).toBe("bad endpoint");
    });
  });

  it("refreshes provider slices after Amp account mutations", async () => {
    let controller: Controller | null = null;
    const ampRow: HarnessAuthRow = {
      key: "amp:amp-2",
      kind: "subscription",
      label: "Amp Two",
      active: false,
      selectable: true,
      account_id: "amp-2",
    };

    vi.mocked(deleteAmpAccount).mockResolvedValue({
      active_account_id: "amp-2",
      accounts: [baseAmpAccounts.accounts[1]!],
    });
    vi.mocked(setAmpActiveAccount).mockResolvedValue({
      active_account_id: "amp-2",
      accounts: baseAmpAccounts.accounts,
    });

    render(createElement(ControllerHarness, {
      onChange: (next) => {
        controller = next;
      },
    }));

    await waitFor(() => {
      expect(controller).not.toBeNull();
    });

    await act(async () => {
      await controller?.onAmpDelete("amp-1");
    });
    await waitFor(() => {
      expect(vi.mocked(listProviders)).toHaveBeenCalledTimes(1);
      expect(vi.mocked(invalidateProvidersBootstrap)).toHaveBeenCalledTimes(1);
    });
    const refreshCallsAfterDelete = vi.mocked(listProviders).mock.calls.length;
    const invalidateCallsAfterDelete = vi.mocked(invalidateProvidersBootstrap).mock.calls.length;

    await act(async () => {
      await controller?.onSelectHarnessAuthRow("amp", ampRow);
    });
    await waitFor(() => {
      expect(vi.mocked(setAmpActiveAccount)).toHaveBeenCalledWith("amp-2");
      expect(vi.mocked(listProviders).mock.calls.length).toBeGreaterThan(refreshCallsAfterDelete);
      expect(vi.mocked(invalidateProvidersBootstrap).mock.calls.length).toBeGreaterThan(invalidateCallsAfterDelete);
    });
  });

  it("cancels an in-flight codex subscription poll when the modal closes", async () => {
    let controller: Controller | null = null;
    const loginPoll = deferred<{ status: "success" }>();

    vi.mocked(startCodexLogin).mockResolvedValue({
      account_id: "codex-login-1",
      auth_url: "https://example.com/codex-login",
      expected_callback_url: null,
      completion_token: "",
    });
    vi.mocked(getCodexLogin).mockReturnValue(loginPoll.promise as ReturnType<typeof getCodexLogin>);
    vi.mocked(openExternalLink).mockResolvedValue(true);

    render(createElement(ControllerHarness, {
      onChange: (next) => {
        controller = next;
      },
    }));

    await waitFor(() => {
      expect(controller).not.toBeNull();
    });

    await act(async () => {
      controller?.openHarnessAuthModal("codex");
    });
    await act(async () => {
      controller?.patchHarnessAuthModal({ stage: "subscription" });
    });

    let submitPromise: Promise<void> | undefined;
    await act(async () => {
      submitPromise = controller?.submitHarnessSubscriptionModal();
    });

    await waitFor(() => {
      expect(vi.mocked(startCodexLogin)).toHaveBeenCalledTimes(1);
      expect(vi.mocked(openExternalLink)).toHaveBeenCalledWith("https://example.com/codex-login");
    });

    await act(async () => {
      controller?.closeHarnessAuthModal();
    });

    loginPoll.resolve({ status: "success" });

    await act(async () => {
      await submitPromise;
    });

    expect(requireController(controller).harnessAuthModal).toBeNull();
    expect(vi.mocked(selectProviderHarnessSource)).not.toHaveBeenCalled();
    expect(requireController(controller).providerError).toBeNull();
  });

  it("suppresses stale subscription completion after switching providers", async () => {
    let controller: Controller | null = null;
    const loginPoll = deferred<{
      login_id: string;
      auth_url?: string | null;
      status: string;
      error?: string | null;
    }>();

    vi.mocked(startGeminiLogin).mockResolvedValue({
      login_id: "gemini-login-1",
      auth_url: "https://example.com/gemini-login",
    });
    vi.mocked(getGeminiLogin).mockReturnValue(loginPoll.promise as ReturnType<typeof getGeminiLogin>);
    vi.mocked(openExternalLink).mockResolvedValue(true);

    render(createElement(ControllerHarness, {
      onChange: (next) => {
        controller = next;
      },
    }));

    await waitFor(() => {
      expect(controller).not.toBeNull();
    });

    await act(async () => {
      controller?.openHarnessAuthModal("gemini");
    });

    let submitPromise: Promise<void> | undefined;
    await act(async () => {
      submitPromise = controller?.submitHarnessSubscriptionModal();
    });

    await waitFor(() => {
      expect(vi.mocked(startGeminiLogin)).toHaveBeenCalledTimes(1);
      expect(vi.mocked(openExternalLink)).toHaveBeenCalledWith("https://example.com/gemini-login");
    });

    await act(async () => {
      controller?.openHarnessAuthModal("qwen");
    });

    loginPoll.resolve({
      login_id: "gemini-login-1",
      auth_url: "https://example.com/gemini-login",
      status: "success",
    });

    await act(async () => {
      await submitPromise;
    });

    expect(requireController(controller).harnessAuthModal?.provider_id).toBe("qwen");
    expect(requireController(controller).harnessAuthModal?.subscription_busy).toBe(false);
    expect(vi.mocked(selectProviderHarnessSource)).not.toHaveBeenCalled();
    expect(requireController(controller).providerError).toBeNull();
  });

  it("suppresses stale api-key submit effects after switching providers", async () => {
    let controller: Controller | null = null;
    const endpointUpsert = deferred<HarnessProviderSourceConfig>();

    vi.mocked(upsertProviderHarnessEndpoint).mockReturnValue(
      endpointUpsert.promise as ReturnType<typeof upsertProviderHarnessEndpoint>,
    );

    render(createElement(ControllerHarness, {
      onChange: (next) => {
        controller = next;
      },
    }));

    await waitFor(() => {
      expect(controller).not.toBeNull();
    });

    await act(async () => {
      controller?.openHarnessAuthModal("codex");
    });
    await act(async () => {
      controller?.patchHarnessAuthModal({
        stage: "api_key",
        api_key: "sk-test",
      });
    });

    let submitPromise: Promise<void> | undefined;
    await act(async () => {
      submitPromise = controller?.submitHarnessApiKeyModal();
    });

    await waitFor(() => {
      expect(vi.mocked(upsertProviderHarnessEndpoint)).toHaveBeenCalledTimes(1);
    });

    await act(async () => {
      controller?.openHarnessAuthModal("gemini");
    });

    endpointUpsert.resolve({
      ...baseCodexConfig,
      endpoints: [
        ...baseCodexConfig.endpoints,
        {
          ...baseEndpoint,
          id: "ep-new",
          name: "Secondary",
          updated_at: "2026-03-03T00:00:00Z",
        },
      ],
    });

    await act(async () => {
      await submitPromise;
    });

    expect(requireController(controller).harnessAuthModal?.provider_id).toBe("gemini");
    expect(vi.mocked(selectProviderHarnessSource)).not.toHaveBeenCalled();
    expect(vi.mocked(verifyProviderForWorkspace)).not.toHaveBeenCalled();
    expect(requireController(controller).providerError).toBeNull();
  });

  it("does not let a stale codex poll close a reopened codex modal", async () => {
    let controller: Controller | null = null;
    const loginPoll = deferred<{ status: "success" }>();

    vi.mocked(startCodexLogin).mockResolvedValue({
      account_id: "codex-login-2",
      auth_url: "https://example.com/codex-login-2",
      expected_callback_url: null,
      completion_token: "",
    });
    vi.mocked(getCodexLogin).mockReturnValue(loginPoll.promise as ReturnType<typeof getCodexLogin>);
    vi.mocked(openExternalLink).mockResolvedValue(true);

    render(createElement(ControllerHarness, {
      onChange: (next) => {
        controller = next;
      },
    }));

    await waitFor(() => {
      expect(controller).not.toBeNull();
    });

    await act(async () => {
      controller?.openHarnessAuthModal("codex");
      controller?.patchHarnessAuthModal({ stage: "subscription" });
    });

    let submitPromise: Promise<void> | undefined;
    await act(async () => {
      submitPromise = controller?.submitHarnessSubscriptionModal();
    });

    await waitFor(() => {
      expect(vi.mocked(startCodexLogin)).toHaveBeenCalledTimes(1);
    });

    await act(async () => {
      controller?.closeHarnessAuthModal();
      controller?.openHarnessAuthModal("codex");
    });

    loginPoll.resolve({ status: "success" });

    await act(async () => {
      await submitPromise;
    });

    expect(requireController(controller).harnessAuthModal?.provider_id).toBe("codex");
    expect(requireController(controller).harnessAuthModal).not.toBeNull();
    expect(vi.mocked(selectProviderHarnessSource)).not.toHaveBeenCalled();
    expect(requireController(controller).providerError).toBeNull();
  });

  it("rolls back endpoint selection silently when verification finishes after switching providers", async () => {
    let controller: Controller | null = null;
    const verifyDeferred = deferred<ProviderAuthCheck>();
    const freshEndpoint = {
      ...baseEndpoint,
      id: "ep-switched",
      name: "Rollback Test",
      updated_at: "2026-03-04T00:00:00Z",
    };
    const afterUpsert: HarnessProviderSourceConfig = {
      ...baseCodexConfig,
      endpoints: [...baseCodexConfig.endpoints, freshEndpoint],
    };
    const selectedEndpointConfig: HarnessProviderSourceConfig = {
      ...afterUpsert,
      selected_source_kind: "endpoint",
      selected_endpoint_id: freshEndpoint.id,
    };

    vi.mocked(upsertProviderHarnessEndpoint).mockResolvedValue(afterUpsert);
    vi.mocked(selectProviderHarnessSource)
      .mockResolvedValueOnce(selectedEndpointConfig)
      .mockResolvedValueOnce(baseCodexConfig);
    vi.mocked(refreshProvidersBootstrap)
      .mockResolvedValueOnce(makeBootstrap({
        provider_harness_config: {
          codex: selectedEndpointConfig,
        },
      }))
      .mockResolvedValueOnce(makeBootstrap());
    vi.mocked(verifyProviderForWorkspace).mockReturnValue(
      verifyDeferred.promise as ReturnType<typeof verifyProviderForWorkspace>,
    );

    render(createElement(ControllerHarness, {
      onChange: (next) => {
        controller = next;
      },
    }));

    await waitFor(() => {
      expect(controller).not.toBeNull();
    });

    await act(async () => {
      controller?.openHarnessAuthModal("codex");
      controller?.patchHarnessAuthModal({
        stage: "api_key",
        api_key: "sk-test",
      });
    });

    let submitPromise: Promise<void> | undefined;
    await act(async () => {
      submitPromise = controller?.submitHarnessApiKeyModal();
    });

    await waitFor(() => {
      expect(vi.mocked(selectProviderHarnessSource)).toHaveBeenNthCalledWith(1, "codex", "endpoint", "ep-switched");
      expect(vi.mocked(verifyProviderForWorkspace)).toHaveBeenCalledWith("ws-test", "codex");
    });

    await act(async () => {
      controller?.openHarnessAuthModal("gemini");
    });

    verifyDeferred.resolve({
      provider_id: "codex",
      workspace_id: "ws-test",
      status: "failed",
      message: "bad endpoint",
    });

    await act(async () => {
      await submitPromise;
    });

    expect(vi.mocked(selectProviderHarnessSource)).toHaveBeenNthCalledWith(2, "codex", "subscription", null);
    expect(requireController(controller).harnessAuthModal?.provider_id).toBe("gemini");
    expect(requireController(controller).providerError).toBeNull();
  });

  it("suppresses duplicate subscription starts before busy state flushes", async () => {
    let controller: Controller | null = null;
    const startAmpLoginDeferred = deferred<{ login_id: string; auth_url?: string | null }>();
    vi.mocked(startAmpLogin).mockReturnValue(
      startAmpLoginDeferred.promise as ReturnType<typeof startAmpLogin>,
    );

    render(createElement(ControllerHarness, {
      onChange: (next) => {
        controller = next;
      },
    }));

    await waitFor(() => {
      expect(controller).not.toBeNull();
    });

    await act(async () => {
      controller?.openHarnessAuthModal("amp");
      controller?.patchHarnessAuthModal({
        stage: "subscription",
        subscription_label: "Amp Login",
      });
    });

    await act(async () => {
      void controller?.submitHarnessSubscriptionModal();
      void controller?.submitHarnessSubscriptionModal();
    });

    expect(vi.mocked(startAmpLogin)).toHaveBeenCalledTimes(1);

    startAmpLoginDeferred.resolve({
      login_id: "amp-login-1",
      auth_url: "https://example.com/amp-login",
    });
  });
});
