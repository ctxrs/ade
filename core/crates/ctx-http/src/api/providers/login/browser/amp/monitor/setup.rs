use super::*;

pub(super) use crate::daemon::providers::PreparedAmpLoginPaths as AmpLoginPaths;

pub(super) async fn prepare_amp_login_paths(
    state: &Arc<AppState>,
    login_id: &str,
) -> Result<AmpLoginPaths, String> {
    crate::daemon::providers::prepare_amp_login_paths(state, login_id).await
}

pub(super) fn amp_provider_env(
    state: &Arc<AppState>,
    amp_home: &StdPath,
) -> HashMap<String, String> {
    crate::daemon::providers::amp_login_provider_env(state, amp_home)
}
