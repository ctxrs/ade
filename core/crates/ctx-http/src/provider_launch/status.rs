use std::sync::Arc;

use anyhow::Context;
use ctx_core::ids::WorkspaceId;
use ctx_provider_install::install_state::InstallTarget;

use crate::daemon::AppState;
use crate::execution_effective;

pub use ctx_provider_runtime::provider_launch::status::provider_status_for_target;

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
