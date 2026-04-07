import type { WorkspaceActiveSnapshotState } from "../../state/workspaceActiveSnapshotStore";
import type { SessionSupervisorSnapshot } from "../../state/sessionSupervisor";
import { useWarmSessionTranscriptRuntimes } from "./useWarmSessionTranscriptRuntimes";
import { useWorkbenchE2EBridge } from "./useWorkbenchE2EBridge";

export function useWorkbenchShellIntegrations({
  workspaceSnapshot,
  sessionSnap,
  activeSessionId,
  focusNewTask,
  clearDraftHarness,
  focusTask,
  toggleDiffPane,
  toggleArtifactsPane,
}: {
  workspaceSnapshot: WorkspaceActiveSnapshotState;
  sessionSnap: SessionSupervisorSnapshot;
  activeSessionId: string | null;
  focusNewTask: () => void;
  clearDraftHarness: () => void;
  focusTask: (taskId: string, sessionId?: string | null) => boolean;
  toggleDiffPane: () => void;
  toggleArtifactsPane: () => void;
}) {
  useWarmSessionTranscriptRuntimes({
    workspaceSnapshot,
    sessionSnap,
    activeSessionId,
  });

  useWorkbenchE2EBridge({
    focusNewTask,
    clearDraftHarness,
    focusTask,
    toggleDiffPane,
    toggleArtifactsPane,
  });
}
