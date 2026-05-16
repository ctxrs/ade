use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use ctx_daemon::daemon::CoreHandle;
use ctx_settings_model as user_settings;
use ctx_settings_service::HostExecutionPolicy;

pub(super) async fn get_settings(
    State(state): State<CoreHandle>,
) -> Result<Json<user_settings::PublicSettings>, StatusCode> {
    let settings = state
        .load_settings()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(state.public_settings_for_response(&settings).await))
}

pub(super) async fn update_settings(
    State(state): State<CoreHandle>,
    Json(req): Json<user_settings::UpdateSettingsReq>,
) -> Result<Json<user_settings::PublicSettings>, StatusCode> {
    let current = state
        .load_settings()
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
    let next = ctx_settings_service::apply_update(current, req);
    state
        .save_settings(&next)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state.apply_settings_side_effects(&next).await;
    Ok(Json(state.public_settings_for_response(&next).await))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let fixture = crate::test_support::TestDaemonFixture::new("http://127.0.0.1:4310").await;
        let req = serde_json::from_value::<user_settings::UpdateSettingsReq>(json!({
            "execution": {
                "mode": "host"
            }
        }))
        .expect("settings update request");

        let err = update_settings(State(fixture.core()), Json(req))
            .await
            .expect_err("sandbox-only policy should reject host execution settings update");

        assert_eq!(err, StatusCode::FORBIDDEN);
    }
}
