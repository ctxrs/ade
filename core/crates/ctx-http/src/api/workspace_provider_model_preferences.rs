use std::sync::Arc;

use anyhow::Result;
use ctx_core::ids::WorkspaceId;

use crate::daemon::AppState;
use ctx_workspace_config as workspace_config;

async fn invalidate_provider_options_cache(
    state: &AppState,
    workspace_id: WorkspaceId,
    provider_id: &str,
) {
    let key_prefix = format!("{}/", workspace_id.0);
    let key_suffix = format!("/{provider_id}");
    state
        .providers
        .options_cache
        .lock()
        .await
        .retain(|key, _| !(key.starts_with(&key_prefix) && key.ends_with(&key_suffix)));
}

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
    invalidate_provider_options_cache(state.as_ref(), workspace_id, provider_id).await;
    Ok(())
}
