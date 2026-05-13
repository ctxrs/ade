use super::*;

pub(super) use crate::daemon::providers::PreparedMistralLoginPaths as MistralLoginPaths;

pub(super) async fn prepare_mistral_login_paths(
    state: &Arc<AppState>,
    login_id: &str,
) -> Result<MistralLoginPaths, String> {
    crate::daemon::providers::prepare_mistral_login_paths(state, login_id).await
}

pub(super) fn mistral_provider_env(
    state: &Arc<AppState>,
    mistral_home: &StdPath,
) -> HashMap<String, String> {
    crate::daemon::providers::mistral_login_provider_env(state, mistral_home)
}
