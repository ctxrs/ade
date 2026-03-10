import { act, render, waitFor } from "@testing-library/react";
import { createElement, useEffect, type Dispatch, type SetStateAction } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DraftHarness } from "../../components/WorkbenchComposer";
import type { ProviderOptions, ProvidersBootstrapResponse } from "../../api/client";
import { getProviderOptions, getProvidersBootstrap } from "../../api/client";
import { refreshProvidersBootstrap } from "../../state/providersBootstrapStore";
import { resolveProviderOptionsUpdate, shouldHydrateProviderModels } from "./useWorkbenchProviders";
import { useWorkbenchProviders } from "./useWorkbenchProviders";

vi.mock("../../api/client", async (importOriginal) => {
  const original = await importOriginal<typeof import("../../api/client")>();
  return {
    ...original,
    getProviderOptions: vi.fn(),
    getProvidersBootstrap: vi.fn(),
  };
});

vi.mock("../../state/providerInstallProgressStore", async (importOriginal) => {
  const original = await importOriginal<typeof import("../../state/providerInstallProgressStore")>();
  return {
    ...original,
    getProviderInstallProgressSnapshot: vi.fn(() => ({})),
    subscribeProviderInstallProgress: vi.fn(() => () => {}),
  };
});

vi.mock("../../state/installProgressMonitor", async (importOriginal) => {
  const original = await importOriginal<typeof import("../../state/installProgressMonitor")>();
  return {
    ...original,
    observeInstall: vi.fn(() => () => {}),
  };
});

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

const requireHookValue = (value: HookValue | null): HookValue => {
  if (!value) {
    throw new Error("hook value not ready");
  }
  return value;
};

const makeBootstrap = (
  providerOptions: Record<string, ProviderOptions>,
): ProvidersBootstrapResponse => ({
  providers: [
    {
      provider_id: "codex",
      display_name: "Codex",
      installed: true,
      health: "ok",
      diagnostics: [],
      details: {},
    } as never,
  ],
  provider_options: providerOptions,
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
});

type HookValue = ReturnType<typeof useWorkbenchProviders>;

const noopSetDraftHarness: Dispatch<SetStateAction<DraftHarness | null>> = () => undefined;

function WorkbenchProvidersHarness({
  workspaceId,
  onChange,
}: {
  workspaceId: string;
  onChange: (value: HookValue) => void;
}) {
  const value = useWorkbenchProviders({
    workspaceId,
    setDraftHarness: noopSetDraftHarness,
    onStartError: () => {},
  });

  useEffect(() => {
    onChange(value);
  }, [onChange, value]);

  return null;
}

beforeEach(() => {
  vi.clearAllMocks();
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

  it("requests hydration for copilot subscription auth when models are missing", () => {
    expect(shouldHydrateProviderModels("copilot", baseOptions("copilot"))).toBe(true);
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

  it("drops preserved models and probe state when subscription account identity changes", () => {
    const previous: ProviderOptions = {
      ...baseOptions("codex"),
      account_identity: "acct-a",
      models: {
        models: [{ id: "gpt-5" }],
        current_model_id: "gpt-5",
      },
      probe_ok: false,
      probe_error: "stale probe failure",
    };
    const next: ProviderOptions = {
      ...baseOptions("codex"),
      account_identity: "acct-b",
      probed_at: "2026-03-09T00:00:05.000Z",
    };

    expect(resolveProviderOptionsUpdate(previous, next)).toEqual(next);
  });

  it("drops preserved models when endpoint credentials rotate on the same endpoint", () => {
    const previous: ProviderOptions = {
      ...baseOptions("claude-crp"),
      auth_mode: "endpoint",
      source: {
        provider_id: "claude-crp",
        selected_source_kind: "endpoint",
        selected_endpoint_id: "ep-1",
        endpoints: [
          {
            id: "ep-1",
            provider_id: "claude-crp",
            name: "Primary",
            base_url: "https://api.example.test",
            api_shape: "anthropic_messages",
            auth_type: "bearer",
            model_override: null,
            created_at: "2026-03-09T00:00:00.000Z",
            updated_at: "2026-03-09T00:00:00.000Z",
            last_verification_status: "valid",
            last_verification_at: null,
            last_error: null,
            has_api_key: true,
          },
        ],
      },
      models: {
        models: [{ id: "claude-sonnet-4.5" }],
        current_model_id: "claude-sonnet-4.5",
      },
    };
    const next: ProviderOptions = {
      ...previous,
      probed_at: "2026-03-09T00:00:05.000Z",
      models: undefined,
      source: {
        ...previous.source!,
        endpoints: [
          {
            ...previous.source!.endpoints[0]!,
            updated_at: "2026-03-09T00:01:00.000Z",
          },
        ],
      },
    };

    expect(resolveProviderOptionsUpdate(previous, next)).toEqual(next);
  });
});

describe("useWorkbenchProviders", () => {
  it("hydrates provider details into the shared scoped resource without losing account identity", async () => {
    const workspaceId = "ws-auth-summary";
    let currentBootstrap = makeBootstrap({
      codex: {
        ...baseOptions("codex"),
        workspace_id: workspaceId,
      },
    });
    let hookValue: HookValue | null = null;

    vi.mocked(getProvidersBootstrap).mockImplementation(async () => currentBootstrap);
    vi.mocked(getProviderOptions).mockResolvedValue({
      ...baseOptions("codex"),
      workspace_id: workspaceId,
      models: {
        models: [{ id: "gpt-5" }],
        current_model_id: "gpt-5",
      },
    });

    render(createElement(WorkbenchProvidersHarness, {
      workspaceId,
      onChange: (next) => {
        hookValue = next;
      },
    }));

    await waitFor(() => {
      expect(hookValue?.providerOptions.codex?.account_identity).toBe("acct-a");
    });

    await act(async () => {
      await hookValue?.ensureProviderAuthSummary("codex");
    });

    await waitFor(() => {
      expect(hookValue?.providerOptions.codex?.models).toEqual({
        models: [{ id: "gpt-5" }],
        current_model_id: "gpt-5",
      });
      expect(hookValue?.providerOptions.codex?.account_identity).toBe("acct-a");
    });

    currentBootstrap = makeBootstrap({
      codex: {
        ...baseOptions("codex"),
        workspace_id: workspaceId,
        probed_at: "2026-03-10T00:00:05.000Z",
      },
    });

    await act(async () => {
      await refreshProvidersBootstrap(workspaceId);
    });

    await waitFor(() => {
      expect(hookValue?.providerOptions.codex?.models).toEqual({
        models: [{ id: "gpt-5" }],
        current_model_id: "gpt-5",
      });
      expect(hookValue?.providerOptions.codex?.account_identity).toBe("acct-a");
    });
  });

  it("drops preserved models when the workspace-scoped auth identity changes", async () => {
    const workspaceId = "ws-auth-identity-change";
    let currentBootstrap = makeBootstrap({
      codex: {
        ...baseOptions("codex"),
        workspace_id: workspaceId,
      },
    });
    let hookValue: HookValue | null = null;

    vi.mocked(getProvidersBootstrap).mockImplementation(async () => currentBootstrap);
    vi.mocked(getProviderOptions).mockResolvedValue({
      ...baseOptions("codex"),
      workspace_id: workspaceId,
      models: {
        models: [{ id: "gpt-5" }],
        current_model_id: "gpt-5",
      },
    });

    render(createElement(WorkbenchProvidersHarness, {
      workspaceId,
      onChange: (next) => {
        hookValue = next;
      },
    }));

    await waitFor(() => {
      expect(hookValue?.providerOptions.codex?.account_identity).toBe("acct-a");
    });

    await act(async () => {
      await hookValue?.ensureProviderAuthSummary("codex");
    });

    await waitFor(() => {
      expect(hookValue?.providerOptions.codex?.models).toBeDefined();
    });

    currentBootstrap = {
      ...makeBootstrap({
        codex: {
          ...baseOptions("codex"),
          workspace_id: workspaceId,
          probed_at: "2026-03-10T00:00:05.000Z",
        },
      }),
      codex_accounts: {
        active_account_id: "acct-b",
        accounts: [],
        logins: [],
      },
    };

    await act(async () => {
      await refreshProvidersBootstrap(workspaceId);
    });

    await waitFor(() => {
      expect(hookValue?.providerOptions.codex?.account_identity).toBe("acct-b");
    });

    expect(requireHookValue(hookValue).providerOptions.codex?.models).toBeUndefined();
  });
});
