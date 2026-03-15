import React from "react";
import { act, render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MessageAttachment } from "../api/client";
import { SessionView } from "./SessionPage";

const paneSpy = vi.hoisted(() => vi.fn());
const sessionEntries = vi.hoisted(() => ({ map: {} as Record<string, unknown> }));

vi.mock("../api/client", () => ({
  deleteMessage: vi.fn(async () => ({})),
  postMessage: vi.fn(async () => ({})),
  setSessionModel: vi.fn(async () => ({})),
  authenticateSession: vi.fn(async () => ({})),
  idToString: (id: string | null | undefined) => {
    if (id == null) return "";
    return String(id);
  },
  interruptSession: vi.fn(async () => ({})),
  submitAskUserQuestion: vi.fn(async () => ({})),
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
    initialData: [],
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
      models: [{ id: "gpt-5.4/medium" }],
    },
  }),
}));

vi.mock("../utils/analytics", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../utils/analytics")>();
  return {
    ...actual,
    useFeatureGate: () => false,
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

const buildAttachment = (blobId: string): MessageAttachment => ({
  kind: "image_ref",
  blob_id: blobId,
  mime_type: "image/png",
  name: `${blobId}.png`,
});

beforeEach(() => {
  paneSpy.mockClear();
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
        reasoning_effort: "medium",
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

describe("SessionPage draft attachments", () => {
  it("renders persisted draft attachments and routes edits back through the draft callbacks", async () => {
    const firstAttachment = buildAttachment("blob-1");
    const secondAttachment = buildAttachment("blob-2");
    const onDraftAttachmentsChange = vi.fn();

    render(
      <SessionView
        sessionId={sessionId}
        draft={{ text: "hello", modeId: "default", attachments: [firstAttachment] }}
        onDraftChange={vi.fn()}
        onDraftAttachmentsChange={onDraftAttachmentsChange}
      />,
    );

    await waitFor(() => {
      expect(paneSpy).toHaveBeenCalled();
    });

    const props = paneSpy.mock.calls.at(-1)?.[0] as
      | {
          draftAttachments: MessageAttachment[];
          setDraftAttachments: React.Dispatch<React.SetStateAction<MessageAttachment[]>>;
        }
      | undefined;

    expect(props?.draftAttachments).toEqual([firstAttachment]);

    await act(async () => {
      props?.setDraftAttachments((prev) => [...prev, secondAttachment]);
    });

    expect(onDraftAttachmentsChange).toHaveBeenCalledWith([firstAttachment, secondAttachment]);
  });
});
