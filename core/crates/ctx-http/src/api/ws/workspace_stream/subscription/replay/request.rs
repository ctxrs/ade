use super::*;
use ctx_workspace_active_snapshot::ResolvedWorkspaceActiveSessionSubscription;

pub(in crate::api::ws::workspace_stream::subscription) struct WorkspaceStreamReplayRequest<'a> {
    pub(in crate::api::ws::workspace_stream::subscription) state: &'a Arc<AppState>,
    pub(in crate::api::ws::workspace_stream::subscription) workspace_id: WorkspaceId,
    pub(in crate::api::ws::workspace_stream::subscription) runtime: &'a mut WorkspaceStreamRuntime,
    pub(in crate::api::ws::workspace_stream::subscription) labels: &'a WorkspaceStreamLabels,
    pub(in crate::api::ws::workspace_stream::subscription) resolved_sessions:
        &'a [ResolvedWorkspaceActiveSessionSubscription],
    pub(in crate::api::ws::workspace_stream::subscription) live_rx:
        &'a mut tokio::sync::broadcast::Receiver<WorkspaceActiveSnapshotEvent>,
    pub(in crate::api::ws::workspace_stream::subscription) include_initial_snapshot: bool,
    pub(in crate::api::ws::workspace_stream::subscription) active_head_cursors:
        &'a HashMap<SessionId, SessionReplayCursor>,
}
