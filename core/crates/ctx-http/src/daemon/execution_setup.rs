use std::sync::Arc;

use tokio::sync::broadcast;

use ctx_core::models::Workspace;
use ctx_execution_runtime::{
    ExecutionLaunchSnapshot, ExecutionLaunchStreamEvent, RuntimePrewarmScope,
    StartupPrewarmSnapshot,
};
use ctx_settings_model::ExecutionSettings;

use crate::daemon::AppState;

pub(crate) async fn launch_status(
    state: &Arc<AppState>,
    job_id: &str,
) -> Option<ExecutionLaunchSnapshot> {
    state.execution.setup.launch_status(job_id).await
}

pub(crate) async fn subscribe_launch(
    state: &Arc<AppState>,
    job_id: &str,
) -> Option<(
    ExecutionLaunchSnapshot,
    broadcast::Receiver<ExecutionLaunchStreamEvent>,
)> {
    state.execution.setup.subscribe_launch(job_id).await
}

pub(crate) async fn start_workspace_launch(
    state: &Arc<AppState>,
    workspace: Workspace,
    execution_settings: ExecutionSettings,
) -> ExecutionLaunchSnapshot {
    state
        .execution
        .setup
        .start_workspace_launch(workspace, execution_settings, state.core.daemon_url.clone())
        .await
}

pub(crate) async fn start_runtime_prewarm(
    state: &Arc<AppState>,
    execution_settings: ExecutionSettings,
    prewarm_scope: RuntimePrewarmScope,
) -> ExecutionLaunchSnapshot {
    state
        .execution
        .setup
        .start_runtime_prewarm(execution_settings, prewarm_scope)
        .await
}

pub(crate) async fn startup_status(state: &Arc<AppState>) -> StartupPrewarmSnapshot {
    state.execution.setup.startup_status().await
}
