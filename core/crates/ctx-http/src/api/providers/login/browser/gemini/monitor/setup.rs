use super::*;

pub(super) use crate::daemon::providers::PreparedGeminiLoginPaths as GeminiLoginPaths;

pub(super) async fn prepare_gemini_login_paths(
    providers: &ProvidersHandle,
    login_id: &str,
) -> Result<GeminiLoginPaths, String> {
    providers.prepare_gemini_login_paths(login_id).await
}

pub(super) fn gemini_provider_env(
    providers: &ProvidersHandle,
    login_home: &StdPath,
) -> HashMap<String, String> {
    providers.gemini_login_provider_env(login_home)
}
