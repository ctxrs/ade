use super::*;

pub(super) use crate::daemon::providers::PreparedGeminiLoginPaths as GeminiLoginPaths;

pub(super) async fn prepare_gemini_login_paths(
    state: &Arc<AppState>,
    login_id: &str,
) -> Result<GeminiLoginPaths, String> {
    crate::daemon::providers::prepare_gemini_login_paths(state, login_id).await
}

pub(super) fn gemini_provider_env(
    state: &AppState,
    login_home: &StdPath,
) -> HashMap<String, String> {
    crate::daemon::providers::gemini_login_provider_env(state, login_home)
}
