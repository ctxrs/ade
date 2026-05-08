use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use crate::daemon::AppState;
use crate::provider_guard;
use crate::provider_restart;
use crate::resource_governance;
use crate::settings as user_settings;
use crate::tool_cgroup;
use ctx_observability::telemetry::TelemetryConfig;
use ctx_settings_service::HostExecutionPolicy;

pub(super) async fn get_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<user_settings::PublicSettings>, StatusCode> {
    let settings = user_settings::load_settings(state.global_store())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut public = user_settings::to_public(&settings);
    public.resource_governance =
        resource_governance::build_public_settings(&state, &settings).await;
    public.tool_limits = tool_cgroup::build_public_settings(&state, &settings).await;
    Ok(Json(public))
}

pub(super) async fn update_settings(
    State(state): State<Arc<AppState>>,
    Json(req): Json<user_settings::UpdateSettingsReq>,
) -> Result<Json<user_settings::PublicSettings>, StatusCode> {
    let current = user_settings::load_settings(state.global_store())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let host_execution_policy =
        HostExecutionPolicy::current().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if req
        .execution
        .as_ref()
        .is_some_and(|execution| matches!(execution.mode, user_settings::ExecutionMode::Host))
    {
        host_execution_policy
            .validate_execution_environment(ctx_core::models::ExecutionEnvironment::Host)
            .map_err(|error| crate::api::shared::status_code_for_request_or_policy_error(&error))?;
    }
    let next = user_settings::apply_update(current, req);
    user_settings::save_settings(state.global_store(), &next)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut telemetry_cfg = TelemetryConfig::default();
    if let Some(telemetry) = next.telemetry.as_ref() {
        telemetry_cfg.enabled = telemetry.enabled;
        if !telemetry.endpoint.trim().is_empty() {
            telemetry_cfg.endpoint = telemetry.endpoint.clone();
        }
    }
    state.telemetry.telemetry.update_config(telemetry_cfg).await;
    let perf_enabled = next.telemetry.as_ref().map(|t| t.enabled).unwrap_or(true);
    state
        .telemetry
        .perf_telemetry
        .update_remote_enabled(perf_enabled)
        .await;
    if let Err(err) = resource_governance::apply_settings(&state, &next).await {
        tracing::warn!("failed to apply resource governance settings: {err:#}");
    }
    if let Err(err) = provider_guard::apply_settings(&state, &next).await {
        tracing::warn!("failed to apply provider guard settings: {err:#}");
    }
    if let Err(err) = provider_restart::apply_settings(&state, &next).await {
        tracing::warn!("failed to apply provider restart settings: {err:#}");
    }
    if let Err(err) = tool_cgroup::apply_settings(&state, &next).await {
        tracing::warn!("failed to apply tool cgroup settings: {err:#}");
    }
    let mut public = user_settings::to_public(&next);
    public.resource_governance = resource_governance::build_public_settings(&state, &next).await;
    public.tool_limits = tool_cgroup::build_public_settings(&state, &next).await;
    Ok(Json(public))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use ctx_store::StoreManager;
    use serde_json::json;

    use ctx_settings_service::EXECUTION_POLICY_TEST_ENV_LOCK;

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.previous {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    #[tokio::test]
    async fn update_settings_rejects_host_execution_when_sandbox_only_policy_is_set() {
        let _env_guard = EXECUTION_POLICY_TEST_ENV_LOCK.lock().await;
        let _policy = EnvVarGuard::set("CTX_HOST_EXECUTION_POLICY", "sandbox_only");
        let _mode = EnvVarGuard::remove("CTX_EXECUTION_MODE");
        let temp = tempfile::tempdir().expect("tempdir");
        let state = Arc::new(AppState::new(
            temp.path().to_path_buf(),
            StoreManager::open(temp.path()).await.expect("open stores"),
            HashMap::new(),
            "http://127.0.0.1:4310".to_string(),
            None,
        ));
        let req = serde_json::from_value::<user_settings::UpdateSettingsReq>(json!({
            "execution": {
                "mode": "host"
            }
        }))
        .expect("settings update request");

        let err = update_settings(State(state), Json(req))
            .await
            .expect_err("sandbox-only policy should reject host execution settings update");

        assert_eq!(err, StatusCode::FORBIDDEN);
    }
}
