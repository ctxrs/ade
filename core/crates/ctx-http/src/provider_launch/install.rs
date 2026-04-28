use std::sync::Arc;

use axum::http::StatusCode;
use axum::Json;

use crate::daemon::AppState;
use crate::logs;
use ctx_provider_install::install_state::{InstallId, InstallTarget};

#[async_trait::async_trait]
impl ctx_provider_runtime::provider_launch::install::ProviderInstallHost for AppState {
    async fn find_running_install(
        &self,
        provider_id: &str,
        target: Option<InstallTarget>,
    ) -> Option<InstallId> {
        AppState::find_running_install(self, provider_id, target).await
    }
}

#[allow(unused_imports)]
pub(crate) use ctx_provider_runtime::provider_launch::install::{
    should_skip_install_for_healthy_provider,
    start_all_provider_installs as runtime_start_all_provider_installs,
};

fn provider_install_error_to_response(
    error: ctx_provider_runtime::provider_launch::install::StartProviderInstallError,
) -> (StatusCode, Json<serde_json::Value>) {
    let status = if error.code.as_deref() == Some("install_target_disabled") {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::BAD_REQUEST
    };
    (
        status,
        Json(serde_json::json!({
            "error": logs::redact_sensitive(&error.message),
            "code": error.code,
        })),
    )
}

pub(crate) async fn start_provider_install(
    state: &Arc<AppState>,
    provider_id: &str,
    target: InstallTarget,
) -> Result<(InstallId, bool), (StatusCode, Json<serde_json::Value>)> {
    ctx_provider_runtime::provider_launch::install::start_provider_install(
        state,
        provider_id,
        target,
    )
    .await
    .map_err(provider_install_error_to_response)
}

pub(crate) async fn start_all_provider_installs(
    state: &Arc<AppState>,
    target: InstallTarget,
) -> Result<Vec<(String, InstallId)>, (StatusCode, Json<serde_json::Value>)> {
    runtime_start_all_provider_installs(state, target)
        .await
        .map_err(provider_install_error_to_response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_install_error_to_response_maps_disabled_install_targets_to_forbidden() {
        let (status, body) = provider_install_error_to_response(
            ctx_provider_runtime::provider_launch::install::StartProviderInstallError {
                message: "host provider installs are disabled by daemon policy".to_string(),
                code: Some("install_target_disabled".to_string()),
            },
        );

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body.0["code"], "install_target_disabled");
    }
}
