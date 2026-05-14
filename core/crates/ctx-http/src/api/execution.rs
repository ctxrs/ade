use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use ctx_execution_runtime::{ExecutionLaunchSnapshot, ExecutionSetupJobKind, RuntimePrewarmScope};

use crate::daemon::{CoreHandle, ExecutionHandle, WorkspacesHandle};
use ctx_observability::logs;
use ctx_settings_model::ExecutionMode;

use super::errors::ApiErrorResp;

mod launch_stream;
mod linux_sandbox;
mod workspace_launch;

pub(super) use launch_stream::{launch_status, launch_stream_ws};
pub(super) use linux_sandbox::{
    linux_sandbox_runtime_prepare, linux_sandbox_runtime_stage, linux_sandbox_runtime_status_api,
};
use workspace_launch::resolve_workspace_launch_inputs;

#[derive(Debug, Deserialize)]
pub(super) struct ExecutionLaunchStartReq {
    #[serde(default)]
    kind: Option<ExecutionSetupJobKind>,
    #[serde(default)]
    workspace_id: Option<String>,
    #[serde(default)]
    prewarm_scope: RuntimePrewarmScope,
}

pub(super) async fn launch_start(
    State(core): State<CoreHandle>,
    State(execution): State<ExecutionHandle>,
    State(workspaces): State<WorkspacesHandle>,
    Json(req): Json<ExecutionLaunchStartReq>,
) -> Result<Json<ExecutionLaunchSnapshot>, (StatusCode, Json<ApiErrorResp>)> {
    execution
        .reject_new_execution_during_maintenance()
        .await
        .map_err(|err| {
            (
                StatusCode::CONFLICT,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&err.to_string()),
                }),
            )
        })?;
    let kind = req.kind.unwrap_or(ExecutionSetupJobKind::WorkspaceLaunch);
    let snapshot = match kind {
        ExecutionSetupJobKind::WorkspaceLaunch => {
            let (workspace, execution_settings) =
                resolve_workspace_launch_inputs(&workspaces, req.workspace_id.as_deref()).await?;
            execution
                .start_workspace_launch(workspace, execution_settings)
                .await
        }
        ExecutionSetupJobKind::StartupPrewarm => {
            let settings = core.load_settings().await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
            let mut execution_settings = settings.execution.unwrap_or_default();
            execution_settings.mode = ExecutionMode::Sandbox;
            execution
                .start_runtime_prewarm(execution_settings, req.prewarm_scope)
                .await
        }
    };
    Ok(Json(snapshot))
}
