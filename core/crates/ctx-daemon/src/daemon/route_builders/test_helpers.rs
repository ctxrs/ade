use super::*;

use crate::daemon::route_builders::state_deps::RouteBuilder;
#[cfg(test)]
use crate::daemon::workspace_route_handles::WorkspacePrimaryBranchRefreshEffect;
#[cfg(test)]
use crate::daemon::workspace_stream_route_handles::WorkspaceVcsStreamRefreshEffect;
use crate::daemon::DaemonState;

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn workspace_attachments_runtime_from_state(
    state: &Arc<DaemonState>,
) -> Arc<crate::daemon::workspaces::attachments::WorkspaceAttachmentsRuntime> {
    let builder = RouteBuilder::new(Arc::clone(state));
    let merge_queue_host = builder.merge_queue_route_host();
    builder
        .workspace_route_deps(merge_queue_host)
        .workspace_attachments_runtime()
}

#[cfg(test)]
pub(crate) fn workspace_primary_branch_with_refresh_effect_from_state(
    state: &Arc<DaemonState>,
    refresh_vcs_snapshot: WorkspacePrimaryBranchRefreshEffect,
) -> WorkspacePrimaryBranchHandle {
    let builder = RouteBuilder::new(Arc::clone(state));
    let merge_queue_host = builder.merge_queue_route_host();
    builder
        .workspace_route_deps(merge_queue_host)
        .workspace_primary_branch_with_refresh_effect(refresh_vcs_snapshot)
}

#[cfg(test)]
pub(crate) fn workspace_vcs_stream_with_refresh_effect_from_state(
    state: &Arc<DaemonState>,
    refresh_worktree_vcs: WorkspaceVcsStreamRefreshEffect,
) -> WorkspaceVcsStreamHandle {
    let builder = RouteBuilder::new(Arc::clone(state));
    let merge_queue_host = builder.merge_queue_route_host();
    builder
        .workspace_route_deps(merge_queue_host)
        .workspace_vcs_stream_with_refresh_effect(refresh_worktree_vcs)
}
