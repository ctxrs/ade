import { useCallback } from "react";
import { copyTextToClipboard } from "../utils/clipboard";
import { SessionView } from "./SessionPage";
import { scrollKey, sessionDraftKey, useWorkbenchDraft, useWorkbenchStore } from "../workbench/store";
import type { WorkbenchScrollState } from "../workbench/types";

export type WorkbenchSessionSlotProps = {
  sessionId: string;
  active: boolean;
  scrollState: WorkbenchScrollState | null;
  preserveScrollOnFocus?: boolean;
  optimisticFailure?: { prompt: string; error: string | null } | null;
};

export function WorkbenchSessionSlot({
  sessionId,
  active,
  scrollState,
  preserveScrollOnFocus,
  optimisticFailure,
}: WorkbenchSessionSlotProps) {
  const workbenchStore = useWorkbenchStore();
  const draft = useWorkbenchDraft(sessionDraftKey(sessionId), { text: "", modeId: "default" });
  const handleScrollStateChange = useCallback(
    (next: { stickToBottom: boolean; anchorItemId: string | null; scrollTop: number | null; virtuosoState?: unknown | null }) => {
      workbenchStore.setScrollState(scrollKey(sessionId), next);
    },
    [sessionId, workbenchStore],
  );

  const onScrollStateChange = active || preserveScrollOnFocus ? handleScrollStateChange : null;

  return (
    <div
      className="wb-session-slot"
      style={{ opacity: active ? 1 : 0, pointerEvents: active ? "auto" : "none" }}
      aria-hidden={!active}
    >
      {optimisticFailure ? (
        <div className="banner" role="alert">
          <div className="row" style={{ justifyContent: "space-between", alignItems: "center" }}>
            <strong>Failed to start</strong>
            <button
              type="button"
              className="wb-link"
              onClick={() => void copyTextToClipboard(optimisticFailure.prompt)}
            >
              Copy prompt
            </button>
          </div>
          {optimisticFailure.error ? (
            <div className="error" style={{ whiteSpace: "pre-wrap" }}>
              {optimisticFailure.error}
            </div>
          ) : null}
        </div>
      ) : null}
      <SessionView
        sessionId={sessionId}
        isActive={active}
        autoOpenSession={false}
        preserveScrollOnFocus={preserveScrollOnFocus}
        draft={draft.value}
        onDraftChange={(text) => draft.setValue({ text, modeId: draft.value.modeId })}
        onDraftPersistNow={() => workbenchStore.flushDraft(sessionDraftKey(sessionId))}
        onModeChange={(modeId) => draft.setValue({ text: draft.value.text, modeId })}
        scrollState={
          scrollState
            ? {
                stickToBottom: scrollState.stickToBottom,
                anchorItemId: scrollState.anchorItemId,
                scrollTop: scrollState.scrollTop ?? null,
                virtuosoState: scrollState.virtuosoState ?? null,
              }
            : null
        }
        onScrollStateChange={onScrollStateChange}
      />
    </div>
  );
}
