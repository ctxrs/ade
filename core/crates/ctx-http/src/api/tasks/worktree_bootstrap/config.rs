use anyhow::Result;
use ctx_core::models::Workspace;
use ctx_workspace_config as workspace_config;
use ctx_workspace_services::worktree_bootstrap::{
    normalize_bootstrap_config, BootstrapConfig, BootstrapConfigInput,
};

use crate::daemon::AppState;

pub(super) async fn load_bootstrap_config(
    state: &AppState,
    workspace: &Workspace,
) -> Result<Option<BootstrapConfig>> {
    let store = state.store_for_workspace(workspace.id).await?;
    let Some(cfg) = workspace_config::load_worktree_bootstrap_config(&store).await? else {
        return Ok(None);
    };

    Ok(normalize_bootstrap_config(BootstrapConfigInput {
        setup_command: cfg.setup_command,
        timeout_sec: cfg.timeout_sec,
        wait_for_completion: cfg.wait_for_completion,
    }))
}
