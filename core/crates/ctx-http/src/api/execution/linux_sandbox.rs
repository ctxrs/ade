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
use crate::daemon::{maintenance as daemon_maintenance, CoreHandle, ExecutionHandle};

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
    State(core): State<CoreHandle>,
) -> Result<Json<LinuxSandboxRuntimeStatus>, (StatusCode, Json<ApiErrorResp>)> {
    let status = linux_sandbox_runtime_status(core.data_root())
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
    State(core): State<CoreHandle>,
) -> Result<Json<LinuxSandboxRuntimeStatus>, (StatusCode, Json<ApiErrorResp>)> {
    let status = stage_linux_sandbox_runtime_downloads(core.data_root(), None)
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
    State(core): State<CoreHandle>,
    State(execution): State<ExecutionHandle>,
    Json(req): Json<LinuxSandboxRuntimePrepareReq>,
) -> Result<Json<LinuxSandboxRuntimePrepareResult>, (StatusCode, Json<ApiErrorResp>)> {
    let drain_permit = execution
        .acquire_linux_sandbox_prepare_drain()
        .await
        .map_err(linux_sandbox_prepare_drain_error)?;
    let result = match prepare_linux_sandbox_runtime(
        core.data_root(),
        req.activation_mode
            .unwrap_or(LinuxSandboxActivationMode::Local),
        req.sudo_password.as_deref(),
        None,
    )
    .await
    {
        Ok(result) => result,
        Err(err) => {
            let _ = drain_permit.release().await;
            tracing::warn!(target: "linux_sandbox", error = %logs::redact_sensitive(&err.to_string()), "linux_sandbox_runtime_prepare error");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: linux_sandbox_user_message("prepare"),
                }),
            ));
        }
    };
    let _ = drain_permit.release().await;
    Ok(Json(result))
}

fn linux_sandbox_prepare_drain_error(
    error: daemon_maintenance::MaintenanceDrainError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        daemon_maintenance::MaintenanceDrainError::AlreadyActive => (
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "Linux sandbox runtime prepare is already in progress. Retry when current maintenance completes.".to_string(),
            }),
        ),
        daemon_maintenance::MaintenanceDrainError::ActivityUnavailable(error) => {
            tracing::warn!(target: "linux_sandbox", error = %logs::redact_sensitive(&error.to_string()), "linux_sandbox_runtime_prepare activity gate error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: linux_sandbox_user_message("prepare"),
                }),
            )
        }
        daemon_maintenance::MaintenanceDrainError::SandboxWorkActive => (
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "Preparing Linux sandbox runtime is blocked while sandbox work is active. Retry when sandbox turns, terminals, containers, and runtime operations are idle.".to_string(),
            }),
        ),
    }
}
