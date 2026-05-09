use anyhow::Context;
use ctx_core::ids::WorkspaceId;

use super::*;
use crate::daemon::execution_effective;

pub(crate) async fn install_target_for_workspace(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> anyhow::Result<InstallTarget> {
    execution_effective::effective_install_target(state.as_ref(), workspace_id)
        .await
        .with_context(|| {
            format!(
                "loading execution settings for workspace {}",
                workspace_id.0
            )
        })
}

pub(in crate::api::providers) fn workspace_execution_settings_error_json(
    error: &anyhow::Error,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "error": format!("failed to load workspace execution settings: {error:#}"),
        })),
    )
}
