use ctx_core::ids::WorkspaceId;
use ctx_provider_install::install_state::InstallTarget;

pub(in crate::api::provider_launch) fn workspace_provider_cache_key(
    workspace_id: WorkspaceId,
    target: InstallTarget,
    provider_id: &str,
) -> String {
    format!("{}/{}/{}", workspace_id.0, target.as_str(), provider_id)
}
