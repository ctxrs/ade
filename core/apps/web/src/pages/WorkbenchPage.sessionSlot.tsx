import { copyTextToClipboard } from "../utils/clipboard";
import { SessionView } from "./SessionPage";
import { sessionDraftKey, useWorkbenchDraft, useWorkbenchStore } from "../workbench/store";

export type WorkbenchSessionSlotProps = {
  sessionId: string;
  active: boolean;
  optimisticFailure?: { prompt: string; error: string | null } | null;
};

export function WorkbenchSessionSlot({
  sessionId,
  active,
  optimisticFailure,
}: WorkbenchSessionSlotProps) {
  const workbenchStore = useWorkbenchStore();
  const draft = useWorkbenchDraft(sessionDraftKey(sessionId), { text: "", modeId: "default" });

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
        draft={draft.value}
        onDraftChange={(text) => draft.setValue({ text, modeId: draft.value.modeId })}
        onDraftPersistNow={() => workbenchStore.flushDraft(sessionDraftKey(sessionId))}
        onModeChange={(modeId) => draft.setValue({ text: draft.value.text, modeId })}
      />
    </div>
  );
}
