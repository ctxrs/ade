use std::sync::Arc;

use anyhow::Result;
use ctx_core::ids::WorkspaceId;

use ctx_provider_runtime::provider_cache;
use ctx_workspace_config as workspace_config;

use crate::daemon::AppState;

pub(crate) async fn update_workspace_provider_preferred_model_id(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    provider_id: &str,
    preferred_model_id: Option<String>,
) -> Result<()> {
    let store = state.store_for_workspace(workspace_id).await?;
    workspace_config::update_preferred_new_session_model_id(
        &store,
        provider_id,
        preferred_model_id,
    )
    .await?;
    provider_cache::invalidate_workspace_provider_options_cache(
        &state.providers,
        workspace_id,
        provider_id,
    )
    .await;
    Ok(())
}
