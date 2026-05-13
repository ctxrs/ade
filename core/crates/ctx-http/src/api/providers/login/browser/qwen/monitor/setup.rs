use super::*;

pub(super) use crate::daemon::providers::PreparedQwenLoginPaths as QwenLoginPaths;

pub(super) async fn prepare_qwen_login_paths(
    state: &Arc<AppState>,
    login_id: &str,
) -> Result<QwenLoginPaths, String> {
    crate::daemon::providers::prepare_qwen_login_paths(state, login_id).await
}

pub(super) fn qwen_provider_env(
    state: &Arc<AppState>,
    login_home: &StdPath,
) -> HashMap<String, String> {
    crate::daemon::providers::qwen_login_provider_env(state, login_home)
}
