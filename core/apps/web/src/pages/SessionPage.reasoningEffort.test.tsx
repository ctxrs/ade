import React from "react";
import { act, render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SessionView } from "./SessionPage";
import { postMessage } from "../api/client";

const paneSpy = vi.hoisted(() => vi.fn());
const sessionEntries = vi.hoisted(() => ({ map: {} as Record<string, unknown> }));
const featureGateMock = vi.hoisted(() => vi.fn(() => false));

vi.mock("../api/client", () => ({
  deleteMessage: vi.fn(async () => ({})),
  postMessage: vi.fn(async () => ({})),
  setSessionModel: vi.fn(async () => ({})),
  authenticateSession: vi.fn(async () => ({})),
  idToString: (id: string | null | undefined) => {
    if (id === null || id === undefined) return "";
    return String(id);
  },
  interruptSession: vi.fn(async () => ({})),
  uploadBlob: vi.fn(async () => ({ blob_id: "blob-1" })),
}));

vi.mock("../state/sessionSupervisor", () => ({
  useSessionSupervisor: () => ({
    refreshQueue: vi.fn(async () => {}),
    refreshSession: vi.fn(async () => {}),
    loadMoreTurns: vi.fn(async () => {}),
    loadTurnTools: vi.fn(async () => {}),
    setSession: vi.fn(),
  }),
  useSessionEntry: (id: string) => sessionEntries.map[id] ?? null,
  useOpenSession: () => {},
}));

vi.mock("../state/uiStateStore", () => ({
  loadSessionViewPrefsV1: vi.fn(async () => null),
  saveSessionViewPrefsV1: vi.fn(async () => {}),
}));

vi.mock("../components/AskUserQuestionCard", () => ({
  AskUserQuestionCard: () => null,
}));

vi.mock("./sessionView/SessionWorkbenchPane", () => ({
  SessionWorkbenchPane: (props: unknown) => {
    paneSpy(props);
    return <div data-testid="session-workbench-pane" />;
  },
}));

vi.mock("./useWorkbenchThreadViewModelController", () => ({
  useWorkbenchThreadViewModelController: () => ({
    view: { debugEvents: [] },
    listItems: [],
  }),
}));

vi.mock("./useSessionMessageListController", () => ({
  useSessionMessageListController: () => ({
    methodsRef: { current: null },
    context: null,
    initialLocation: null,
    onScroll: vi.fn(),
    onRenderedDataChange: vi.fn(),
  }),
}));

vi.mock("./sessionView/useSessionImageDropScope", () => ({
  useSessionImageDropScope: () => ({
    dropScopeRef: { current: null },
    dropActive: false,
  }),
}));

vi.mock("./sessionView/useSessionProviderGuard", () => ({
  useSessionProviderGuard: () => ({
    providerGuardActionError: null,
    providerGuardActionBusy: false,
    providerGuardMemoryLimitMb: null,
    providerGuardHeading: "",
    providerGuardMessage: "",
    providerGuardLimitLabel: "",
    providerGuardProviderLabel: "",
    providerGuardPidLabel: "",
    canRaiseProviderGuard: false,
    raiseProviderGuardLimit: vi.fn(async () => {}),
    disableProviderGuard: vi.fn(async () => {}),
  }),
}));

vi.mock("./sessionView/useStableAskUserQuestionAnswers", () => ({
  useStableAskUserQuestionAnswers: () => ({}),
}));

vi.mock("./sessionView/useSharedSessionProviderOptions", () => ({
  useSharedSessionProviderOptions: () => ({
    provider_id: "codex",
    workspace_id: "ws-1",
    supports_load: false,
    auth_required: false,
    has_active_auth: true,
    auth_mode: "subscription",
    probed_at: "2026-03-10T00:00:00.000Z",
    models: {
      current_model_id: "gpt-5.4/medium",
      models: [{ id: "gpt-5.4/medium" }, { id: "gpt-5.4/xhigh" }],
    },
  }),
}));

vi.mock("../utils/analytics", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../utils/analytics")>();
  return {
    ...actual,
    useFeatureGate: featureGateMock,
  };
});

vi.mock("../utils/useDictationController", () => ({
  useDictationController: () => ({
    dictationRecording: false,
    dictationError: null,
    dictationDebugText: "",
    dictationOnboarding: null,
    dismissDictationOnboarding: vi.fn(),
    backDictationOnboarding: vi.fn(),
    chooseDictationOnboardingLocal: vi.fn(),
    chooseDictationOnboardingCloud: vi.fn(),
    updateDictationOnboardingCloud: vi.fn(),
    submitDictationOnboardingLocal: vi.fn(),
    submitDictationOnboardingCloud: vi.fn(),
    startDictation: vi.fn(async () => {}),
    stopDictation: vi.fn(async () => ""),
  }),
}));

vi.mock("../workbench/store", () => ({
  useWorkbenchStore: () => ({
    focusTask: vi.fn(),
  }),
}));

const sessionId = "session-1";
const postMessageMock = vi.mocked(postMessage);

beforeEach(() => {
  paneSpy.mockClear();
  postMessageMock.mockClear();
  featureGateMock.mockReset();
  featureGateMock.mockReturnValue(false);
  sessionEntries.map = {
    [sessionId]: {
      sessionId,
      session: {
        id: sessionId,
        task_id: "task-1",
        workspace_id: "ws-1",
        worktree_id: "wt-1",
        provider_id: "codex",
        model_id: "gpt-5.4",
        reasoning_effort: "xhigh",
        title: "Session 1",
        agent_role: "assistant",
        status: "active",
        execution_environment: "host",
        created_at: "2026-03-10T00:00:00.000Z",
        updated_at: "2026-03-10T00:00:00.000Z",
      },
      acpCurrentModelId: "gpt-5.4/medium",
      turns: [],
      turnToolsByTurnId: {},
      turnToolsLoading: [],
      toolSummariesReady: true,
      hasMoreTurns: false,
      events: [],
      messages: [],
      artifacts: [],
      artifactsLoading: false,
      subagentInvocations: [],
      subagentInvocationsLoading: false,
      stateLoaded: true,
      stateLoading: false,
      turnsRev: 0,
      messagesRev: 0,
      eventsRev: 0,
      queue: [],
      loading: false,
      subscribed: true,
      updatedAtMs: 0,
    },
  };
});

describe("SessionPage reasoning effort", () => {
  it("uses session-owned reasoning effort for the active session selector instead of ACP current model defaults", async () => {
    render(<SessionView sessionId={sessionId} />);

    await waitFor(() => {
      expect(paneSpy).toHaveBeenCalled();
    });

    const lastCall = paneSpy.mock.calls.at(-1)?.[0] as {
      currentModelId?: string;
      availableModels?: Array<{ id: string }>;
    } | undefined;
    expect(lastCall?.currentModelId).toBe("gpt-5.4/xhigh");
    expect(lastCall?.availableModels?.map((model) => model.id)).toEqual([
      "gpt-5.4/medium",
      "gpt-5.4/xhigh",
    ]);
  });

  it("lets send reach the daemon when local active-turn state is only bootstrap", async () => {
    const existingEntry = (sessionEntries.map[sessionId] ?? {}) as Record<string, unknown>;
    sessionEntries.map[sessionId] = {
      ...existingEntry,
      freshness: "bootstrap",
      activity: { is_working: true, last_turn_status: "running" },
    };

    render(<SessionView sessionId={sessionId} />);

    await waitFor(() => {
      expect(paneSpy).toHaveBeenCalled();
    });

    await act(async () => {
      const props = paneSpy.mock.calls.at(-1)?.[0] as { setInput: (value: string) => void };
      props.setInput("hello from bootstrap");
    });

    await act(async () => {
      const props = paneSpy.mock.calls.at(-1)?.[0] as { sendNow: () => Promise<void> };
      await props.sendNow();
    });

    expect(postMessageMock).toHaveBeenCalledTimes(1);
    expect(postMessageMock.mock.calls[0]?.[0]).toBe(sessionId);
    expect(postMessageMock.mock.calls[0]?.[1]).toBe("hello from bootstrap");
    expect(postMessageMock.mock.calls[0]?.[2]).toBeUndefined();
  });

  it("does not force queued delivery when active-turn state is only bootstrap", async () => {
    featureGateMock.mockReturnValue(true);

    const existingEntry = (sessionEntries.map[sessionId] ?? {}) as Record<string, unknown>;
    sessionEntries.map[sessionId] = {
      ...existingEntry,
      freshness: "bootstrap",
      activity: { is_working: true, last_turn_status: "running" },
    };

    render(<SessionView sessionId={sessionId} />);

    await waitFor(() => {
      expect(paneSpy).toHaveBeenCalled();
    });

    await act(async () => {
      const props = paneSpy.mock.calls.at(-1)?.[0] as { setInput: (value: string) => void };
      props.setInput("hello from bootstrap queue gate");
    });

    await act(async () => {
      const props = paneSpy.mock.calls.at(-1)?.[0] as { sendNow: () => Promise<void> };
      await props.sendNow();
    });

    expect(postMessageMock).toHaveBeenCalledTimes(1);
    expect(postMessageMock.mock.calls[0]?.[1]).toBe("hello from bootstrap queue gate");
    expect(postMessageMock.mock.calls[0]?.[2]).toBeUndefined();
  });

  it("queues delivery locally when the running turn is authoritative", async () => {
    featureGateMock.mockReturnValue(true);

    const existingEntry = (sessionEntries.map[sessionId] ?? {}) as Record<string, unknown>;
    sessionEntries.map[sessionId] = {
      ...existingEntry,
      freshness: "authoritative",
      activity: { is_working: true, last_turn_status: "running" },
    };

    render(<SessionView sessionId={sessionId} />);

    await waitFor(() => {
      expect(paneSpy).toHaveBeenCalled();
    });

    await act(async () => {
      const props = paneSpy.mock.calls.at(-1)?.[0] as { setInput: (value: string) => void };
      props.setInput("hello from authoritative queue gate");
    });

    await act(async () => {
      const props = paneSpy.mock.calls.at(-1)?.[0] as { sendNow: () => Promise<void> };
      await props.sendNow();
    });

    expect(postMessageMock).toHaveBeenCalledTimes(1);
    expect(postMessageMock.mock.calls[0]?.[1]).toBe("hello from authoritative queue gate");
    expect(postMessageMock.mock.calls[0]?.[2]).toBe("queued");
  });
});
