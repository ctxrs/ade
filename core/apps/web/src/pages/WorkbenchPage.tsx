import { useEffect } from "react";
import { useParams } from "react-router-dom";
import { WorkbenchStoreProvider } from "../workbench/store";
import { WorkspaceActiveSnapshotProvider } from "../state/workspaceActiveSnapshotStore";
import { WorkbenchPageInner } from "./WorkbenchPage.shell";
import { trackFeatureUsed, trackWorkspaceOpened } from "../utils/analytics";

export { TaskRow } from "./WorkbenchPage.taskRow";

export default function WorkbenchPage() {
  const { id: workspaceId } = useParams<{ id: string }>();
  useEffect(() => {
    if (!workspaceId) return;
    trackWorkspaceOpened("local");
    trackFeatureUsed("workbench_opened");
  }, [workspaceId]);
  if (!workspaceId) return null;
  return (
    <WorkspaceActiveSnapshotProvider workspaceId={workspaceId}>
      <WorkbenchStoreProvider workspaceId={workspaceId}>
        <WorkbenchPageInner workspaceId={workspaceId} />
      </WorkbenchStoreProvider>
    </WorkspaceActiveSnapshotProvider>
  );
}
