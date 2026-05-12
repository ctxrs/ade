use super::health::{build_health_response, HealthResp};
use super::*;
use ctx_linux_sandbox_runtime::linux_sandbox_runtime_status;
use ctx_provider_runtime::provider_launch::status::mark_provider_status_with_managed_config_error;

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
    let startup_prewarm = state.execution.setup.startup_status().await;
    let linux_sandbox_runtime = linux_sandbox_runtime_status(&state.core.data_root)
        .await
        .map(|status| serde_json::to_value(status).unwrap_or_else(|_| serde_json::json!({})))
        .unwrap_or_else(
            |err| serde_json::json!({"error": logs::redact_sensitive(&err.to_string())}),
        );
    let (managed_installs, managed_config_error) =
        match installer::load_agent_server_config(&state.core.data_root).await {
            Ok(cfg) => (
                serde_json::to_value(cfg).unwrap_or_else(|_| serde_json::json!({})),
                None,
            ),
            Err(err) => {
                let error = logs::redact_sensitive(&err.to_string());
                (serde_json::json!({ "error": error }), Some(error))
            }
        };
    let managed_installs = redact_json_value(managed_installs);

    let providers = {
        let mut providers = state
            .providers
            .with_provider_statuses(|map| map.values().cloned().collect::<Vec<_>>())
            .await;
        if let Some(config_error) = managed_config_error.as_deref() {
            for status in &mut providers {
                mark_provider_status_with_managed_config_error(status, config_error);
            }
        }
        providers
            .into_iter()
            .map(|mut s| {
                s.diagnostics = s
                    .diagnostics
                    .into_iter()
                    .map(|d| logs::redact_sensitive(&d))
                    .collect();
                s.details = s
                    .details
                    .into_iter()
                    .filter(|(k, _)| !is_sensitive_key(k))
                    .map(|(k, v)| (k, logs::redact_sensitive(&v)))
                    .collect();
                s
            })
            .collect::<Vec<_>>()
    };

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
        providers,
        managed_installs,
    }))
}
