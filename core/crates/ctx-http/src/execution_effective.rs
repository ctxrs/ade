use ctx_core::ids::WorkspaceId;

use crate::daemon::AppState;
use crate::settings::ExecutionSettings;
use crate::{settings, workspace_config};

/// Compute effective execution settings for a workspace, combining daemon defaults with any
/// workspace runtime override.
pub async fn effective_execution_settings(
    state: &AppState,
    workspace_id: WorkspaceId,
) -> anyhow::Result<ExecutionSettings> {
    let settings_data = settings::load_settings(state.global_store()).await?;
    let mut effective = settings_data.execution.clone().unwrap_or_default();
    let store = state.store_for_workspace(workspace_id).await?;
    if let Some(ov) = workspace_config::load_execution_settings_override(&store).await? {
        workspace_config::apply_execution_settings_override(&mut effective, &ov);
    }
    Ok(effective)
}
