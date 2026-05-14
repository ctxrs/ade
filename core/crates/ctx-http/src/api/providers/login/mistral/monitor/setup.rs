use super::*;

pub(super) use ctx_daemon::daemon::providers::PreparedMistralLoginPaths as MistralLoginPaths;

pub(super) async fn prepare_mistral_login_paths(
    providers: &ProvidersHandle,
    login_id: &str,
) -> Result<MistralLoginPaths, String> {
    providers.prepare_mistral_login_paths(login_id).await
}

pub(super) fn mistral_provider_env(
    providers: &ProvidersHandle,
    mistral_home: &StdPath,
) -> HashMap<String, String> {
    providers.mistral_login_provider_env(mistral_home)
}
