use super::health::{build_health_response, HealthResp};
use super::*;
use ctx_linux_sandbox_runtime::linux_sandbox_runtime_status;

#[derive(Debug, Serialize)]
pub(in crate::api) struct DiagnosticsResp {
    daemon: HealthResp,
    platform: serde_json::Value,
    logs: serde_json::Value,
    execution: serde_json::Value,
    providers: Vec<ProviderStatus>,
    managed_installs: serde_json::Value,
}

pub(in crate::api) async fn diagnostics(
    State(state): State<Arc<AppState>>,
) -> Result<Json<DiagnosticsResp>, StatusCode> {
    let startup_prewarm = crate::daemon::execution_setup::startup_status(&state).await;
    let linux_sandbox_runtime = linux_sandbox_runtime_status(&state.core.data_root)
        .await
        .map(|status| serde_json::to_value(status).unwrap_or_else(|_| serde_json::json!({})))
        .unwrap_or_else(
            |err| serde_json::json!({"error": logs::redact_sensitive(&err.to_string())}),
        );
    let provider_diagnostics =
        crate::daemon::providers::provider_diagnostics_snapshot(&state).await;

    let log_files = logs::list_log_files(&state.core.data_root).await;

    let identity = ctx_update_service::current_build_identity(env!("CARGO_PKG_VERSION"))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(DiagnosticsResp {
        daemon: build_health_response(&state, identity, true),
        platform: serde_json::json!({
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        }),
        logs: serde_json::json!({
            "dir": logs::logs_dir(&state.core.data_root).to_string_lossy(),
            "files": log_files,
        }),
        execution: serde_json::json!({
            "startup_prewarm": startup_prewarm,
            "linux_sandbox_runtime": linux_sandbox_runtime,
        }),
        providers: provider_diagnostics.providers,
        managed_installs: provider_diagnostics.managed_installs,
    }))
}
