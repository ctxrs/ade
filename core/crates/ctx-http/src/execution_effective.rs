use std::path::Path;

use crate::settings::ExecutionSettings;
use crate::{settings, workspace_config};

/// Compute effective execution settings for a workspace, combining daemon defaults with any
/// workspace `.ctx/config.toml` override.
pub async fn effective_execution_settings(
    data_root: &Path,
    workspace_root: &Path,
) -> ExecutionSettings {
    let settings_data = settings::load_settings(data_root).await;
    let mut effective = settings_data.execution.clone().unwrap_or_default();
    if let Ok(Some(ov)) = workspace_config::load_execution_settings_override(workspace_root).await {
        workspace_config::apply_execution_settings_override(&mut effective, &ov);
    }
    effective
}
