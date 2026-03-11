import { act, render, waitFor } from "@testing-library/react";
import { Fragment, createElement, useEffect } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  AmpAccountsResponse,
  HarnessProviderSourceConfig,
  ProvidersBootstrapResponse,
  ProviderAuthCheck,
  ProviderStatus,
} from "../../../api/client";
import {
  deleteAmpAccount,
  getCodexLogin,
  getGeminiLogin,
  getProviderOptions,
  selectProviderHarnessSource,
  setAmpActiveAccount,
  setCodexActiveAccount,
  startAmpLogin,
  startCodexLogin,
  startGeminiLogin,
  upsertProviderHarnessEndpoint,
  verifyProviderForWorkspace,
} from "../../../api/client";
import {
  invalidateHostProvidersBootstrap,
  invalidateProvidersBootstrap,
  loadHostProvidersBootstrap,
  loadProvidersBootstrap,
  refreshHostProvidersBootstrap,
  refreshProvidersBootstrap,
  refreshProvidersBootstrapForScope,
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
import { resetProviderOnboardingCoordinatorForTests } from "../../../state/providerOnboardingCoordinator";
import type { HarnessAuthRow } from "../harnessAuthRows";
import { openExternalLink } from "../../../utils/desktop";

const bootstrapMockState = vi.hoisted(() => ({
  bootstrapStateByWorkspace: new Map<string, ProvidersBootstrapResponse>(),
  bootstrapLoadQueueByWorkspace: new Map<string, Array<ProvidersBootstrapResponse | Error>>(),
  bootstrapRefreshQueueByWorkspace: new Map<string, Array<ProvidersBootstrapResponse | Error>>(),
  bootstrapListenersByWorkspace: new Map<string, Set<() => void>>(),
  hostBootstrapState: null as ProvidersBootstrapResponse | null,
  hostBootstrapLoadQueue: [] as Array<ProvidersBootstrapResponse | Error>,
  hostBootstrapRefreshQueue: [] as Array<ProvidersBootstrapResponse | Error>,
  hostBootstrapListeners: new Set<() => void>(),
}));

const getBootstrapListeners = (workspaceId: string): Set<() => void> => {
  let listeners = bootstrapMockState.bootstrapListenersByWorkspace.get(workspaceId);
  if (!listeners) {
    listeners = new Set();
    bootstrapMockState.bootstrapListenersByWorkspace.set(workspaceId, listeners);
  }
  return listeners;
};

const setBootstrapSnapshot = (workspaceId: string, next: ProvidersBootstrapResponse): ProvidersBootstrapResponse => {
  bootstrapMockState.bootstrapStateByWorkspace.set(workspaceId, next);
  for (const listener of getBootstrapListeners(workspaceId)) {
    listener();
  }
  return next;
};

const queueBootstrapLoad = (workspaceId: string, ...entries: Array<ProvidersBootstrapResponse | Error>): void => {
  bootstrapMockState.bootstrapLoadQueueByWorkspace.set(workspaceId, entries);
};

const queueBootstrapRefresh = (workspaceId: string, ...entries: Array<ProvidersBootstrapResponse | Error>): void => {
  bootstrapMockState.bootstrapRefreshQueueByWorkspace.set(workspaceId, entries);
};

const consumeBootstrapQueue = (
  workspaceId: string,
  queueByWorkspace: Map<string, Array<ProvidersBootstrapResponse | Error>>,
  empty: ProvidersBootstrapResponse,
): ProvidersBootstrapResponse => {
  const queue = queueByWorkspace.get(workspaceId);
  if (queue && queue.length > 0) {
    const next = queue.shift()!;
    if (queue.length === 0) {
      queueByWorkspace.delete(workspaceId);
    }
    if (next instanceof Error) {
      throw next;
    }
    return setBootstrapSnapshot(workspaceId, next);
  }
  return bootstrapMockState.bootstrapStateByWorkspace.get(workspaceId) ?? empty;
};

const setHostBootstrapSnapshot = (next: ProvidersBootstrapResponse): ProvidersBootstrapResponse => {
  bootstrapMockState.hostBootstrapState = next;
  for (const listener of bootstrapMockState.hostBootstrapListeners) {
    listener();
  }
  return next;
};

const queueHostBootstrapLoad = (...entries: Array<ProvidersBootstrapResponse | Error>): void => {
  bootstrapMockState.hostBootstrapLoadQueue = [...entries];
};

const queueHostBootstrapRefresh = (...entries: Array<ProvidersBootstrapResponse | Error>): void => {
  bootstrapMockState.hostBootstrapRefreshQueue = [...entries];
};

const consumeHostBootstrapQueue = (
  queueKey: "hostBootstrapLoadQueue" | "hostBootstrapRefreshQueue",
  empty: ProvidersBootstrapResponse,
): ProvidersBootstrapResponse => {
  const queue = bootstrapMockState[queueKey];
  if (queue.length > 0) {
    const next = queue.shift()!;
    if (next instanceof Error) {
      throw next;
    }
    return setHostBootstrapSnapshot(next);
  }
  return bootstrapMockState.hostBootstrapState ?? empty;
};

vi.mock("../../../api/client", async (importOriginal) => {
  const original = await importOriginal<typeof import("../../../api/client")>();
  return {
    ...original,
    deleteAmpAccount: vi.fn(),
    getCodexLogin: vi.fn(),
    getGeminiLogin: vi.fn(),
    getProviderOptions: vi.fn(),
    selectProviderHarnessSource: vi.fn(),
    setAmpActiveAccount: vi.fn(),
    setCodexActiveAccount: vi.fn(),
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
    getHostProvidersBootstrapSnapshot: vi.fn(() =>
      bootstrapMockState.hostBootstrapState ?? original.EMPTY_PROVIDERS_BOOTSTRAP),
    getProvidersBootstrapSnapshot: vi.fn((workspaceId: string) =>
      bootstrapMockState.bootstrapStateByWorkspace.get(workspaceId) ?? original.EMPTY_PROVIDERS_BOOTSTRAP),
    getProvidersBootstrapSnapshotForScope: vi.fn((ownerScope: { kind: "host" | "workspace"; workspaceId?: string }) =>
      ownerScope.kind === "workspace"
        ? bootstrapMockState.bootstrapStateByWorkspace.get(ownerScope.workspaceId ?? "") ?? original.EMPTY_PROVIDERS_BOOTSTRAP
        : bootstrapMockState.hostBootstrapState ?? original.EMPTY_PROVIDERS_BOOTSTRAP),
    hasCachedHostProvidersBootstrap: vi.fn(() => bootstrapMockState.hostBootstrapState !== null),
    invalidateHostProvidersBootstrap: vi.fn(),
    invalidateProvidersBootstrap: vi.fn(),
    loadHostProvidersBootstrap: vi.fn(async () =>
      consumeHostBootstrapQueue("hostBootstrapLoadQueue", original.EMPTY_PROVIDERS_BOOTSTRAP)),
    loadProvidersBootstrap: vi.fn(async (workspaceId: string) =>
      consumeBootstrapQueue(workspaceId, bootstrapMockState.bootstrapLoadQueueByWorkspace, original.EMPTY_PROVIDERS_BOOTSTRAP)),
    loadProvidersBootstrapForScope: vi.fn(async (ownerScope: { kind: "host" | "workspace"; workspaceId?: string }) =>
      ownerScope.kind === "workspace"
        ? consumeBootstrapQueue(
          ownerScope.workspaceId ?? "",
          bootstrapMockState.bootstrapLoadQueueByWorkspace,
          original.EMPTY_PROVIDERS_BOOTSTRAP,
        )
        : consumeHostBootstrapQueue("hostBootstrapLoadQueue", original.EMPTY_PROVIDERS_BOOTSTRAP)),
    refreshHostProvidersBootstrap: vi.fn(async () =>
      consumeHostBootstrapQueue("hostBootstrapRefreshQueue", original.EMPTY_PROVIDERS_BOOTSTRAP)),
    refreshProvidersBootstrap: vi.fn(async (workspaceId: string) =>
      consumeBootstrapQueue(workspaceId, bootstrapMockState.bootstrapRefreshQueueByWorkspace, original.EMPTY_PROVIDERS_BOOTSTRAP)),
    refreshProvidersBootstrapForScope: vi.fn(async (ownerScope: { kind: "host" | "workspace"; workspaceId?: string }) =>
      ownerScope.kind === "workspace"
        ? consumeBootstrapQueue(
          ownerScope.workspaceId ?? "",
          bootstrapMockState.bootstrapRefreshQueueByWorkspace,
          original.EMPTY_PROVIDERS_BOOTSTRAP,
        )
        : consumeHostBootstrapQueue("hostBootstrapRefreshQueue", original.EMPTY_PROVIDERS_BOOTSTRAP)),
    subscribeHostProvidersBootstrap: vi.fn((listener: () => void) => {
      bootstrapMockState.hostBootstrapListeners.add(listener);
      return () => {
        bootstrapMockState.hostBootstrapListeners.delete(listener);
      };
    }),
    subscribeProvidersBootstrap: vi.fn((workspaceId: string, listener: () => void) => {
      const listeners = getBootstrapListeners(workspaceId);
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    }),
    subscribeProvidersBootstrapForScope: vi.fn((ownerScope: { kind: "host" | "workspace"; workspaceId?: string }, listener: () => void) => {
      if (ownerScope.kind === "workspace") {
        const listeners = getBootstrapListeners(ownerScope.workspaceId ?? "");
        listeners.add(listener);
        return () => {
          listeners.delete(listener);
        };
      }
      bootstrapMockState.hostBootstrapListeners.add(listener);
      return () => {
        bootstrapMockState.hostBootstrapListeners.delete(listener);
      };
    }),
    updateHostProvidersBootstrap: vi.fn((updater: (current: ProvidersBootstrapResponse) => ProvidersBootstrapResponse) =>
      setHostBootstrapSnapshot(
        updater(bootstrapMockState.hostBootstrapState ?? original.EMPTY_PROVIDERS_BOOTSTRAP),
      )),
    updateProvidersBootstrap: vi.fn((workspaceId: string, updater: (current: ProvidersBootstrapResponse) => ProvidersBootstrapResponse) =>
      setBootstrapSnapshot(
        workspaceId,
        updater(bootstrapMockState.bootstrapStateByWorkspace.get(workspaceId) ?? original.EMPTY_PROVIDERS_BOOTSTRAP),
      )),
    updateProvidersBootstrapForScope: vi.fn((
      ownerScope: { kind: "host" | "workspace"; workspaceId?: string },
      updater: (current: ProvidersBootstrapResponse) => ProvidersBootstrapResponse,
    ) => ownerScope.kind === "workspace"
      ? setBootstrapSnapshot(
        ownerScope.workspaceId ?? "",
        updater(bootstrapMockState.bootstrapStateByWorkspace.get(ownerScope.workspaceId ?? "") ?? original.EMPTY_PROVIDERS_BOOTSTRAP),
      )
      : setHostBootstrapSnapshot(
        updater(bootstrapMockState.hostBootstrapState ?? original.EMPTY_PROVIDERS_BOOTSTRAP),
      )),
  };
});

vi.mock("../../../state/providerInstallProgressStore", async (importOriginal) => {
  const original = await importOriginal<typeof import("../../../state/providerInstallProgressStore")>();
  return {
    ...original,
    getProviderInstallProgressSnapshot: vi.fn(() => ({})),
    getProviderInstallProgressSnapshotForScope: vi.fn(() => ({})),
    subscribeProviderInstallProgress: vi.fn(() => () => {}),
    subscribeProviderInstallProgressForScope: vi.fn(() => () => {}),
    upsertProviderInstallProgress: vi.fn(),
    upsertProviderInstallProgressForScope: vi.fn(),
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

function ControllerHarness({
  onChange,
  workspaceId = "ws-test",
}: {
  onChange: (controller: Controller) => void;
  workspaceId?: string | null;
}) {
  const controller = useHarnessAuthenticationController({
    workspaceId,
    enabled: true,
  });

  useEffect(() => {
    onChange(controller);
  }, [controller, onChange]);

  return null;
}

beforeEach(() => {
  resetProviderOnboardingCoordinatorForTests();
  bootstrapMockState.bootstrapStateByWorkspace.clear();
  bootstrapMockState.bootstrapLoadQueueByWorkspace.clear();
  bootstrapMockState.bootstrapRefreshQueueByWorkspace.clear();
  bootstrapMockState.bootstrapListenersByWorkspace.clear();
  bootstrapMockState.hostBootstrapState = null;
  bootstrapMockState.hostBootstrapLoadQueue = [];
  bootstrapMockState.hostBootstrapRefreshQueue = [];
  bootstrapMockState.hostBootstrapListeners.clear();
  vi.clearAllMocks();
  vi.mocked(deleteAmpAccount).mockReset();
  vi.mocked(getCodexLogin).mockReset();
  vi.mocked(getGeminiLogin).mockReset();
  vi.mocked(getProviderOptions).mockReset();
  vi.mocked(selectProviderHarnessSource).mockReset();
  vi.mocked(setAmpActiveAccount).mockReset();
  vi.mocked(setCodexActiveAccount).mockReset();
  vi.mocked(startAmpLogin).mockReset();
  vi.mocked(startCodexLogin).mockReset();
  vi.mocked(startGeminiLogin).mockReset();
  vi.mocked(upsertProviderHarnessEndpoint).mockReset();
  vi.mocked(verifyProviderForWorkspace).mockReset();
  vi.mocked(invalidateHostProvidersBootstrap).mockReset();
  vi.mocked(invalidateProvidersBootstrap).mockReset();
  vi.mocked(loadHostProvidersBootstrap).mockReset();
  vi.mocked(loadProvidersBootstrap).mockReset();
  vi.mocked(refreshHostProvidersBootstrap).mockReset();
  vi.mocked(refreshProvidersBootstrap).mockReset();
  vi.mocked(refreshProvidersBootstrapForScope).mockReset();
  vi.mocked(openExternalLink).mockReset();
  setBootstrapSnapshot("ws-test", makeBootstrap());
  setHostBootstrapSnapshot(makeBootstrap());
  vi.mocked(loadHostProvidersBootstrap).mockImplementation(async () =>
    consumeHostBootstrapQueue("hostBootstrapLoadQueue", makeBootstrap()));
  vi.mocked(loadProvidersBootstrap).mockImplementation(async (workspaceId: string) =>
    consumeBootstrapQueue(workspaceId, bootstrapMockState.bootstrapLoadQueueByWorkspace, makeBootstrap()));
  vi.mocked(refreshHostProvidersBootstrap).mockImplementation(async () =>
    consumeHostBootstrapQueue("hostBootstrapRefreshQueue", makeBootstrap()));
  vi.mocked(refreshProvidersBootstrap).mockImplementation(async (workspaceId: string) =>
    consumeBootstrapQueue(workspaceId, bootstrapMockState.bootstrapRefreshQueueByWorkspace, makeBootstrap()));
  vi.mocked(refreshProvidersBootstrapForScope).mockImplementation(async (
    ownerScope: { kind: "host" | "workspace"; workspaceId?: string },
  ) => ownerScope.kind === "workspace"
    ? consumeBootstrapQueue(
      ownerScope.workspaceId ?? "",
      bootstrapMockState.bootstrapRefreshQueueByWorkspace,
      makeBootstrap(),
    )
    : consumeHostBootstrapQueue("hostBootstrapRefreshQueue", makeBootstrap()));
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
    expect(supportsHarnessSubscriptionAuth("mistral")).toBe(false);
  });

  it("returns true for subscription-capable providers", () => {
    expect(supportsHarnessSubscriptionAuth("codex")).toBe(true);
    expect(supportsHarnessSubscriptionAuth("claude-crp")).toBe(true);
    expect(supportsHarnessSubscriptionAuth("gemini")).toBe(true);
  });

  it("keeps cursor API-key-only until managed browser auth is mainlined", () => {
    expect(supportsHarnessSubscriptionAuth("cursor")).toBe(false);
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
  });

  it("routes cursor directly to api_key until managed browser auth is ported", () => {
    expect(resolveHarnessAuthModalInitialStage("cursor")).toBe("api_key");
  });
});

describe("useHarnessAuthenticationController", () => {
  it("submits Gemini Vertex service-account endpoint auth without a base URL", async () => {
    let controller: Controller | null = null;
    const vertexEndpoint = {
      ...baseEndpoint,
      id: "gemini-vertex-1",
      provider_id: "gemini",
      name: "Gemini Vertex",
      base_url: null,
      auth_type: "vertex_ai",
      model_override: null,
    };
    const selectedEndpointConfig: HarnessProviderSourceConfig = {
      provider_id: "gemini",
      selected_source_kind: "endpoint",
      selected_endpoint_id: vertexEndpoint.id,
      endpoints: [vertexEndpoint],
    };

    setBootstrapSnapshot("ws-test", makeBootstrap({
      provider_harness_config: {
        codex: baseCodexConfig,
        gemini: selectedEndpointConfig,
      },
    }));
    queueBootstrapRefresh("ws-test", makeBootstrap({
      provider_harness_config: {
        codex: baseCodexConfig,
        gemini: selectedEndpointConfig,
      },
    }));
    vi.mocked(upsertProviderHarnessEndpoint).mockResolvedValue(selectedEndpointConfig);
    vi.mocked(selectProviderHarnessSource).mockResolvedValue(selectedEndpointConfig);
    vi.mocked(verifyProviderForWorkspace).mockResolvedValue({
      provider_id: "gemini",
      workspace_id: "ws-test",
      status: "ok",
      message: undefined,
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
      controller?.openHarnessAuthModal("gemini");
    });
    await waitFor(() => {
      expect(controller?.harnessAuthModal?.provider_id).toBe("gemini");
      expect(controller?.harnessAuthModal?.base_url).toBe("");
    });

    await act(async () => {
      controller?.patchHarnessAuthModal({
        stage: "api_key",
        endpoint_provider_id: "google_vertex",
        gemini_endpoint_auth_type: "vertex_ai",
        service_account_json:
          '{"type":"service_account","project_id":"vertex-project","private_key_id":"key-id","private_key":"-----BEGIN PRIVATE KEY-----\\nabc\\n-----END PRIVATE KEY-----\\n","client_email":"ctx-vertex@test.iam.gserviceaccount.com","client_id":"1234567890"}',
        project_id: "vertex-project",
        location: "global",
        base_url: "not_used",
      });
    });
    await act(async () => {
      await controller?.submitHarnessApiKeyModal();
    });

    await waitFor(() => {
      expect(vi.mocked(upsertProviderHarnessEndpoint)).toHaveBeenCalledWith("gemini", expect.objectContaining({
        base_url: null,
        auth_type: "vertex_ai",
        service_account_json: expect.stringContaining('"type":"service_account"'),
        project_id: "vertex-project",
        location: "global",
      }));
    });
  });

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
    queueBootstrapRefresh(
      "ws-test",
      makeBootstrap({
        provider_harness_config: {
          codex: selectedEndpointConfig,
        },
      }),
      makeBootstrap(),
    );
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
    queueBootstrapRefresh("ws-test", makeBootstrap({
      providers: [
        {
          provider_id: "codex",
          display_name: "Codex",
          installed: true,
          health: "ok",
          details: {
            install_target: "container",
          },
        } as never,
      ],
    }));

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
      expect(vi.mocked(refreshProvidersBootstrapForScope)).toHaveBeenCalledTimes(1);
      expect(vi.mocked(invalidateProvidersBootstrap)).toHaveBeenCalledTimes(1);
      expect(requireController(controller).providers[0]?.details?.install_target).toBe("container");
    });
    const refreshCallsAfterDelete = vi.mocked(refreshProvidersBootstrapForScope).mock.calls.length;
    const invalidateCallsAfterDelete = vi.mocked(invalidateProvidersBootstrap).mock.calls.length;

    await act(async () => {
      await controller?.onSelectHarnessAuthRow("amp", ampRow);
    });
    await waitFor(() => {
      expect(vi.mocked(setAmpActiveAccount)).toHaveBeenCalledWith("amp-2");
      expect(vi.mocked(refreshProvidersBootstrapForScope).mock.calls.length).toBeGreaterThan(refreshCallsAfterDelete);
      expect(vi.mocked(invalidateProvidersBootstrap).mock.calls.length).toBeGreaterThan(invalidateCallsAfterDelete);
      expect(requireController(controller).providers[0]?.details?.install_target).toBe("container");
    });
  });

  it("warms Codex provider options after a workspace-scoped auth change", async () => {
    let controller: Controller | null = null;
    const codexRow: HarnessAuthRow = {
      key: "codex:acct-next",
      kind: "subscription",
      label: "Codex Next",
      active: false,
      selectable: true,
      account_id: "acct-next",
    };
    const pinnedCodexOptions = {
      provider_id: "codex",
      workspace_id: "ws-test",
      supports_load: false,
      auth_required: false,
      has_active_auth: true,
      auth_mode: "subscription" as const,
      source: {
        provider_id: "codex",
        selected_source_kind: "subscription" as const,
        selected_endpoint_id: null,
        endpoints: [],
      },
      probed_at: "2026-03-10T00:00:00.000Z",
      models: {
        models: [{ id: "gpt-5.3-codex/low" }, { id: "gpt-5.3-codex/medium" }],
        current_model_id: "gpt-5.3-codex/medium",
        meta: {
          source_kind: "subscription",
          catalog_source: "codex_bundle_pinned",
          refresh_pending: true,
        },
      },
    };

    setBootstrapSnapshot("ws-test", makeBootstrap({
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
      provider_options: {
        codex: pinnedCodexOptions,
      },
      codex_accounts: {
        active_account_id: "acct-current",
        accounts: [
          { id: "acct-current", label: "Current", created_at: "2026-03-10T00:00:00.000Z" },
          { id: "acct-next", label: "Next", created_at: "2026-03-10T00:00:00.000Z" },
        ],
        logins: [],
      },
    }));
    queueBootstrapRefresh("ws-test", makeBootstrap({
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
      provider_options: {
        codex: pinnedCodexOptions,
      },
      codex_accounts: {
        active_account_id: "acct-next",
        accounts: [
          { id: "acct-current", label: "Current", created_at: "2026-03-10T00:00:00.000Z" },
          { id: "acct-next", label: "Next", created_at: "2026-03-10T00:00:00.000Z" },
        ],
        logins: [],
      },
    }));
    vi.mocked(setCodexActiveAccount).mockResolvedValue({
      active_account_id: "acct-next",
      accounts: [
        { id: "acct-current", label: "Current", created_at: "2026-03-10T00:00:00.000Z" },
        { id: "acct-next", label: "Next", created_at: "2026-03-10T00:00:00.000Z" },
      ],
      logins: [],
    });
    vi.mocked(getProviderOptions).mockResolvedValue({
      ...pinnedCodexOptions,
      models: {
        models: [{ id: "gpt-5.4/low" }, { id: "gpt-5.4/medium" }],
        current_model_id: "gpt-5.4/medium",
      },
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
      await controller?.onSelectHarnessAuthRow("codex", codexRow);
    });

    await waitFor(() => {
      expect(vi.mocked(setCodexActiveAccount)).toHaveBeenCalledWith("acct-next");
      expect(vi.mocked(getProviderOptions)).toHaveBeenCalledWith("ws-test", "codex");
    });
  });

  it("preserves workspace-scoped providers when workspace refresh fails after a mutation", async () => {
    let controller: Controller | null = null;
    const workspaceProvider: ProviderStatus = {
      provider_id: "codex",
      installed: true,
      health: "ok",
      diagnostics: [],
      details: {
        install_target: "container",
      },
    };

    setBootstrapSnapshot("ws-test", makeBootstrap({
      providers: [workspaceProvider],
    }));
    queueBootstrapRefresh("ws-test", new Error("workspace refresh failed"));
    vi.mocked(deleteAmpAccount).mockResolvedValue({
      active_account_id: "amp-2",
      accounts: [baseAmpAccounts.accounts[1]!],
    });

    render(createElement(ControllerHarness, {
      onChange: (next) => {
        controller = next;
      },
    }));

    await waitFor(() => {
      expect(controller).not.toBeNull();
      expect(requireController(controller).providers[0]?.details?.install_target).toBe("container");
    });

    await act(async () => {
      await controller?.onAmpDelete("amp-1");
    });

    await waitFor(() => {
      expect(requireController(controller).providerError).toBe("workspace refresh failed");
    });

    expect(requireController(controller).providers[0]?.details?.install_target).toBe("container");
    expect(vi.mocked(loadHostProvidersBootstrap)).not.toHaveBeenCalled();
    expect(vi.mocked(refreshHostProvidersBootstrap)).not.toHaveBeenCalled();
    expect(vi.mocked(refreshProvidersBootstrapForScope)).toHaveBeenCalledWith(expect.objectContaining({
      kind: "workspace",
      workspaceId: "ws-test",
    }));
    expect(vi.mocked(invalidateHostProvidersBootstrap)).not.toHaveBeenCalled();
    expect(vi.mocked(invalidateProvidersBootstrap)).toHaveBeenCalledWith("ws-test");
  });

  it("shares host-scoped mutations through the host bootstrap store without touching workspace scope", async () => {
    let firstController: Controller | null = null;
    let secondController: Controller | null = null;
    const hostProvider: ProviderStatus = {
      provider_id: "codex",
      installed: true,
      health: "ok",
      diagnostics: [],
      details: {
        install_target: "host",
      },
    };

    setHostBootstrapSnapshot(makeBootstrap({
      providers: [hostProvider],
    }));
    vi.mocked(deleteAmpAccount).mockResolvedValue({
      active_account_id: "amp-2",
      accounts: [baseAmpAccounts.accounts[1]!],
    });

    render(createElement(Fragment, {}, [
      createElement(ControllerHarness, {
        key: "first",
        workspaceId: null,
        onChange: (next) => {
          firstController = next;
        },
      }),
      createElement(ControllerHarness, {
        key: "second",
        workspaceId: null,
        onChange: (next) => {
          secondController = next;
        },
      }),
    ]));

    await waitFor(() => {
      expect(firstController).not.toBeNull();
      expect(secondController).not.toBeNull();
      expect(requireController(firstController).providers[0]?.details?.install_target).toBe("host");
      expect(requireController(secondController).ampAccounts?.active_account_id).toBe("amp-1");
    });

    await act(async () => {
      await firstController?.onAmpDelete("amp-1");
    });

    await waitFor(() => {
      expect(requireController(firstController).ampAccounts?.active_account_id).toBe("amp-2");
      expect(requireController(secondController).ampAccounts?.active_account_id).toBe("amp-2");
    });

    expect(vi.mocked(invalidateHostProvidersBootstrap)).toHaveBeenCalledTimes(1);
    expect(vi.mocked(refreshProvidersBootstrapForScope)).toHaveBeenCalledWith(expect.objectContaining({
      kind: "host",
    }));
    expect(vi.mocked(refreshHostProvidersBootstrap)).not.toHaveBeenCalled();
    expect(vi.mocked(invalidateProvidersBootstrap)).not.toHaveBeenCalled();
    expect(vi.mocked(refreshProvidersBootstrap)).not.toHaveBeenCalled();
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
    queueBootstrapRefresh(
      "ws-test",
      makeBootstrap({
        provider_harness_config: {
          codex: selectedEndpointConfig,
        },
      }),
      makeBootstrap(),
    );
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

    await act(async () => {
      startAmpLoginDeferred.resolve({
        login_id: "amp-login-1",
        auth_url: "https://example.com/amp-login",
      });
      await Promise.resolve();
    });
  });
});
