use ctx_daemon::daemon::workspaces::stream::WorkspaceVcsDemandState;
use ctx_daemon::daemon::WorkspacesHandle;

pub(in crate::api::ws::workspace_vcs) type WorkspaceVcsRuntime = WorkspaceVcsDemandState;

pub(in crate::api::ws::workspace_vcs) async fn release_workspace_vcs_demand(
    state: &WorkspacesHandle,
    runtime: &WorkspaceVcsRuntime,
) {
    state.release_workspace_vcs_demand(runtime).await;
}
