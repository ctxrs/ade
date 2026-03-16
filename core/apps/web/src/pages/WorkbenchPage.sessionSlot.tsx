import { copyTextToClipboard } from "../utils/clipboard";
import { SessionView } from "./SessionPage";
import { sessionDraftKey, useWorkbenchDraft, useWorkbenchStore } from "../workbench/store";

export type WorkbenchSessionSlotProps = {
  sessionId: string;
  optimisticFailure?: { prompt: string; error: string | null } | null;
};

export function WorkbenchSessionSlot({
  sessionId,
  optimisticFailure,
}: WorkbenchSessionSlotProps) {
  const workbenchStore = useWorkbenchStore();
  const draft = useWorkbenchDraft(sessionDraftKey(sessionId), { text: "", modeId: "default", attachments: [] });

  return (
    <div className="wb-session-slot" aria-hidden="false">
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
        autoOpenSession={false}
        draft={draft.value}
        onDraftChange={(text) => draft.setValue((prev) => ({ ...prev, text }))}
        onDraftAttachmentsChange={(attachments) => draft.setValue((prev) => ({ ...prev, attachments }))}
        onDraftPersistNow={() => workbenchStore.flushDraft(sessionDraftKey(sessionId))}
        onModeChange={(modeId) => draft.setValue((prev) => ({ ...prev, modeId }))}
      />
    </div>
  );
}
