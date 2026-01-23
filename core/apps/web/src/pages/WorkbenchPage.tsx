import { useParams } from "react-router-dom";
import { WorkbenchStoreProvider } from "../workbench/store";
import { WorkspaceActiveSnapshotProvider } from "../state/workspaceActiveSnapshotStore";
import { WorkbenchPageInner } from "./WorkbenchPage.shell";

export { TaskRow } from "./WorkbenchPage.taskRow";

export default function WorkbenchPage() {
  const { id: workspaceId } = useParams<{ id: string }>();
  if (!workspaceId) return null;
  return (
    <WorkspaceActiveSnapshotProvider workspaceId={workspaceId}>
      <WorkbenchStoreProvider workspaceId={workspaceId}>
        <WorkbenchPageInner workspaceId={workspaceId} />
      </WorkbenchStoreProvider>
    </WorkspaceActiveSnapshotProvider>
  );
}
