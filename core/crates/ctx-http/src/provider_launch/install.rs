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
    should_skip_install_for_healthy_provider, start_all_provider_installs,
};

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
    .map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": logs::redact_sensitive(&error.message),
                "code": error.code,
            })),
        )
    })
}
