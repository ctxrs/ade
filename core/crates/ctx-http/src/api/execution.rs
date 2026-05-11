use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use ctx_execution_runtime::{ExecutionLaunchSnapshot, ExecutionSetupJobKind, RuntimePrewarmScope};

use crate::daemon::AppState;
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
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExecutionLaunchStartReq>,
) -> Result<Json<ExecutionLaunchSnapshot>, (StatusCode, Json<ApiErrorResp>)> {
    state
        .core
        .update_drain
        .reject_if_draining()
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
                resolve_workspace_launch_inputs(&state, req.workspace_id.as_deref()).await?;
            state
                .execution
                .setup
                .start_workspace_launch(
                    workspace,
                    execution_settings,
                    state.core.daemon_url.clone(),
                )
                .await
        }
        ExecutionSetupJobKind::StartupPrewarm => {
            let settings = ctx_settings_service::load_settings(state.global_store())
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?;
            let mut execution_settings = settings.execution.unwrap_or_default();
            execution_settings.mode = ExecutionMode::Sandbox;
            state
                .execution
                .setup
                .start_runtime_prewarm(execution_settings, req.prewarm_scope)
                .await
        }
    };
    Ok(Json(snapshot))
}
