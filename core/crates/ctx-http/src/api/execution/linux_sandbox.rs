use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use ctx_linux_sandbox_runtime::{
    linux_sandbox_runtime_status, prepare_linux_sandbox_runtime,
    stage_linux_sandbox_runtime_downloads, LinuxSandboxActivationMode,
    LinuxSandboxRuntimePrepareResult, LinuxSandboxRuntimeStatus,
};
use ctx_observability::logs;

use crate::api::errors::ApiErrorResp;
use crate::daemon::AppState;

fn linux_sandbox_user_message(kind: &str) -> String {
    match kind {
        "status" => "Linux sandbox runtime status check failed".to_string(),
        "stage" => "Linux sandbox runtime downloads failed to stage".to_string(),
        "prepare" => "Preparing Linux sandbox runtime failed".to_string(),
        _ => "Linux sandbox operation failed".to_string(),
    }
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct LinuxSandboxRuntimePrepareReq {
    #[serde(default)]
    activation_mode: Option<LinuxSandboxActivationMode>,
    #[serde(default)]
    sudo_password: Option<String>,
}

pub(in crate::api) async fn linux_sandbox_runtime_status_api(
    State(state): State<Arc<AppState>>,
) -> Result<Json<LinuxSandboxRuntimeStatus>, (StatusCode, Json<ApiErrorResp>)> {
    let status = linux_sandbox_runtime_status(&state.core.data_root)
        .await
        .map_err(|err| {
            tracing::warn!(target: "linux_sandbox", error = %logs::redact_sensitive(&err.to_string()), "linux_sandbox_runtime_status_api error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp { error: linux_sandbox_user_message("status") }),
            )
        })?;
    Ok(Json(status))
}

pub(in crate::api) async fn linux_sandbox_runtime_stage(
    State(state): State<Arc<AppState>>,
) -> Result<Json<LinuxSandboxRuntimeStatus>, (StatusCode, Json<ApiErrorResp>)> {
    let status = stage_linux_sandbox_runtime_downloads(&state.core.data_root, None)
        .await
        .map_err(|err| {
            tracing::warn!(target: "linux_sandbox", error = %logs::redact_sensitive(&err.to_string()), "linux_sandbox_runtime_stage error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp { error: linux_sandbox_user_message("stage") }),
            )
        })?;
    Ok(Json(status))
}

pub(in crate::api) async fn linux_sandbox_runtime_prepare(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LinuxSandboxRuntimePrepareReq>,
) -> Result<Json<LinuxSandboxRuntimePrepareResult>, (StatusCode, Json<ApiErrorResp>)> {
    if state
        .core
        .update_drain
        .acquire("linux_sandbox_runtime_prepare", "execution_api")
        .await
        .is_none()
    {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "Linux sandbox runtime prepare is already in progress. Retry when current maintenance completes.".to_string(),
            }),
        ));
    }
    let activity = crate::daemon::daemon_sandbox_work_activity_summary(&state)
        .await
        .map_err(|err| {
            let state = state.clone();
            tokio::spawn(async move {
                let _ = state.core.update_drain.release().await;
            });
            tracing::warn!(target: "linux_sandbox", error = %logs::redact_sensitive(&err.to_string()), "linux_sandbox_runtime_prepare activity gate error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp { error: linux_sandbox_user_message("prepare") }),
            )
        })?;
    if activity.active {
        let _ = state.core.update_drain.release().await;
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "Preparing Linux sandbox runtime is blocked while sandbox work is active. Retry when sandbox turns, terminals, containers, and runtime operations are idle.".to_string(),
            }),
        ));
    }
    let result = prepare_linux_sandbox_runtime(
        &state.core.data_root,
        req.activation_mode
            .unwrap_or(LinuxSandboxActivationMode::Local),
        req.sudo_password.as_deref(),
        None,
    )
    .await
    .map_err(|err| {
        let state = state.clone();
        tokio::spawn(async move {
            let _ = state.core.update_drain.release().await;
        });
        tracing::warn!(target: "linux_sandbox", error = %logs::redact_sensitive(&err.to_string()), "linux_sandbox_runtime_prepare error");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp { error: linux_sandbox_user_message("prepare") }),
        )
    })?;
    let _ = state.core.update_drain.release().await;
    Ok(Json(result))
}
