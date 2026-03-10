import { act, render, waitFor } from "@testing-library/react";
import { Fragment, createElement, useEffect } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ProviderOptions, ProvidersBootstrapResponse } from "../api/client";
import { getProviderOptions, getProvidersBootstrap } from "../api/client";
import { observeInstall } from "./installProgressMonitor";
import {
  clearProviderInstallProgress,
  upsertProviderInstallProgress,
} from "./providerInstallProgressStore";
import {
  resetProviderOnboardingCoordinatorForTests,
  useProviderOnboardingCoordinator,
} from "./providerOnboardingCoordinator";

vi.mock("../api/client", async (importOriginal) => {
  const original = await importOriginal<typeof import("../api/client")>();
  return {
    ...original,
    getProviderOptions: vi.fn(),
    getProvidersBootstrap: vi.fn(),
  };
});

vi.mock("./installProgressMonitor", async (importOriginal) => {
  const original = await importOriginal<typeof import("./installProgressMonitor")>();
  return {
    ...original,
    observeInstall: vi.fn(() => () => {}),
  };
});

type HookValue = ReturnType<typeof useProviderOnboardingCoordinator>;

const baseOptions = (workspaceId: string, providerId: string): ProviderOptions => ({
  provider_id: providerId,
  workspace_id: workspaceId,
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
  probed_at: "2026-03-10T00:00:00.000Z",
});

const makeBootstrap = (
  workspaceId: string,
  overrides?: Partial<ProvidersBootstrapResponse>,
): ProvidersBootstrapResponse => ({
  providers: [
    {
      provider_id: "codex",
      display_name: "Codex",
      installed: false,
      health: "ok",
      diagnostics: [],
      details: {
        install_id: "install-codex",
        install_running: "true",
        install_target: "container",
      },
    } as never,
  ],
  provider_options: {
    codex: baseOptions(workspaceId, "codex"),
  },
  provider_harness_config: {},
  codex_accounts: {
    active_account_id: "acct-a",
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
  amp_accounts: {
    active_account_id: null,
    accounts: [],
  },
  ...overrides,
});

function CoordinatorHarness({
  workspaceId,
  onChange,
}: {
  workspaceId: string;
  onChange: (value: HookValue) => void;
}) {
  const value = useProviderOnboardingCoordinator({
    workspaceId,
    enabled: true,
  });

  useEffect(() => {
    onChange(value);
  }, [onChange, value]);

  return null;
}

beforeEach(() => {
  vi.clearAllMocks();
  resetProviderOnboardingCoordinatorForTests();
  clearProviderInstallProgress();
});

describe("providerOnboardingCoordinator", () => {
  it("shares running-install observation and foreground refresh across duplicate workspace subscribers", async () => {
    const workspaceId = "ws-shared-provider-onboarding";
    const stopInstallObservation = vi.fn();
    let firstValue: HookValue | null = null;
    let secondValue: HookValue | null = null;

    vi.mocked(observeInstall).mockReturnValue(stopInstallObservation);
    vi.mocked(getProvidersBootstrap).mockImplementation(async () => makeBootstrap(workspaceId));

    render(createElement(Fragment, {}, [
      createElement(CoordinatorHarness, {
        key: "first",
        workspaceId,
        onChange: (value) => {
          firstValue = value;
        },
      }),
      createElement(CoordinatorHarness, {
        key: "second",
        workspaceId,
        onChange: (value) => {
          secondValue = value;
        },
      }),
    ]));

    await waitFor(() => {
      expect(firstValue?.bootstrap.providers[0]?.details?.install_id).toBe("install-codex");
      expect(secondValue?.bootstrap.providers[0]?.details?.install_id).toBe("install-codex");
      expect(vi.mocked(observeInstall)).toHaveBeenCalledTimes(1);
      expect(vi.mocked(getProvidersBootstrap)).toHaveBeenCalledTimes(1);
    });

    vi.mocked(getProvidersBootstrap).mockClear();

    await act(async () => {
      window.dispatchEvent(new Event("focus"));
    });

    await waitFor(() => {
      expect(vi.mocked(getProvidersBootstrap)).toHaveBeenCalledTimes(1);
    });
  });

  it("refreshes bootstrap and hydrates provider options after a succeeded install", async () => {
    const workspaceId = "ws-post-install-followup";
    const stopInstallObservation = vi.fn();
    let currentBootstrap = makeBootstrap(workspaceId);
    let hookValue: HookValue | null = null;

    vi.mocked(observeInstall).mockReturnValue(stopInstallObservation);
    vi.mocked(getProvidersBootstrap).mockImplementation(async () => currentBootstrap);
    vi.mocked(getProviderOptions).mockResolvedValue({
      ...baseOptions(workspaceId, "codex"),
      models: {
        models: [{ id: "gpt-5" }],
        current_model_id: "gpt-5",
      },
    });

    render(createElement(CoordinatorHarness, {
      workspaceId,
      onChange: (value) => {
        hookValue = value;
      },
    }));

    await waitFor(() => {
      expect(hookValue?.bootstrap.providers[0]?.details?.install_running).toBe("true");
      expect(vi.mocked(observeInstall)).toHaveBeenCalledTimes(1);
    });

    currentBootstrap = makeBootstrap(workspaceId, {
      providers: [
        {
          provider_id: "codex",
          display_name: "Codex",
          installed: true,
          health: "ok",
          diagnostics: [],
          details: {
            install_id: "install-codex",
            install_running: "false",
            install_target: "container",
          },
        } as never,
      ],
    });

    act(() => {
      upsertProviderInstallProgress("codex", {
        installId: "install-codex",
        state: "succeeded",
        pct: 100,
        target: "container",
      });
    });

    await waitFor(() => {
      expect(vi.mocked(getProviderOptions)).toHaveBeenCalledWith(workspaceId, "codex");
      expect(hookValue?.bootstrap.provider_options.codex?.models).toEqual({
        models: [{ id: "gpt-5" }],
        current_model_id: "gpt-5",
      });
      expect(stopInstallObservation).toHaveBeenCalled();
    });
  });
});
