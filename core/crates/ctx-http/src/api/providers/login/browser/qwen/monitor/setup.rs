use super::*;

pub(super) use ctx_daemon::daemon::providers::PreparedQwenLoginPaths as QwenLoginPaths;

pub(super) async fn prepare_qwen_login_paths(
    providers: &ProvidersHandle,
    login_id: &str,
) -> Result<QwenLoginPaths, String> {
    providers.prepare_qwen_login_paths(login_id).await
}

pub(super) fn qwen_provider_env(
    providers: &ProvidersHandle,
    login_home: &StdPath,
) -> HashMap<String, String> {
    providers.qwen_login_provider_env(login_home)
}
