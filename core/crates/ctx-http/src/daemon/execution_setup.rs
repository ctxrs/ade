use std::sync::Arc;

use tokio::sync::broadcast;

use ctx_core::models::Workspace;
use ctx_execution_runtime::{
    ExecutionLaunchSnapshot, ExecutionLaunchStreamEvent, RuntimePrewarmScope,
    StartupPrewarmSnapshot,
};
use ctx_settings_model::ExecutionSettings;

use crate::daemon::{DaemonState, ExecutionHandle};

pub(crate) async fn launch_status(
    state: &Arc<DaemonState>,
    job_id: &str,
) -> Option<ExecutionLaunchSnapshot> {
    state.execution.setup.launch_status(job_id).await
}

pub(crate) async fn subscribe_launch(
    state: &Arc<DaemonState>,
    job_id: &str,
) -> Option<(
    ExecutionLaunchSnapshot,
    broadcast::Receiver<ExecutionLaunchStreamEvent>,
)> {
    state.execution.setup.subscribe_launch(job_id).await
}

pub(crate) async fn start_workspace_launch(
    state: &Arc<DaemonState>,
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
    state: &Arc<DaemonState>,
    execution_settings: ExecutionSettings,
    prewarm_scope: RuntimePrewarmScope,
) -> ExecutionLaunchSnapshot {
    state
        .execution
        .setup
        .start_runtime_prewarm(execution_settings, prewarm_scope)
        .await
}

pub(crate) async fn startup_status(state: &Arc<DaemonState>) -> StartupPrewarmSnapshot {
    state.execution.setup.startup_status().await
}

impl ExecutionHandle {
    pub(crate) async fn launch_status(&self, job_id: &str) -> Option<ExecutionLaunchSnapshot> {
        launch_status(&self.state, job_id).await
    }

    pub(crate) async fn subscribe_launch(
        &self,
        job_id: &str,
    ) -> Option<(
        ExecutionLaunchSnapshot,
        broadcast::Receiver<ExecutionLaunchStreamEvent>,
    )> {
        subscribe_launch(&self.state, job_id).await
    }

    pub(crate) async fn start_workspace_launch(
        &self,
        workspace: Workspace,
        execution_settings: ExecutionSettings,
    ) -> ExecutionLaunchSnapshot {
        start_workspace_launch(&self.state, workspace, execution_settings).await
    }

    pub(crate) async fn start_runtime_prewarm(
        &self,
        execution_settings: ExecutionSettings,
        prewarm_scope: RuntimePrewarmScope,
    ) -> ExecutionLaunchSnapshot {
        start_runtime_prewarm(&self.state, execution_settings, prewarm_scope).await
    }

    pub(crate) async fn startup_status(&self) -> StartupPrewarmSnapshot {
        startup_status(&self.state).await
    }
}
