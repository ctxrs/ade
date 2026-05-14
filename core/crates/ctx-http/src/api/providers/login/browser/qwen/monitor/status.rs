use super::*;

pub(super) async fn set_failed(providers: &ProvidersHandle, login_id: &str, error: String) {
    providers.set_qwen_login_failed(login_id, error).await;
}

pub(super) async fn set_failed_if_no_error(
    providers: &ProvidersHandle,
    login_id: &str,
    error: String,
) {
    providers
        .set_qwen_login_failed_if_no_error(login_id, error)
        .await;
}

pub(super) async fn set_timeout_if_no_error(
    providers: &ProvidersHandle,
    login_id: &str,
    error: String,
) {
    providers
        .set_qwen_login_timeout_if_no_error(login_id, error)
        .await;
}

pub(super) async fn set_auth_url(providers: &ProvidersHandle, login_id: &str, auth_url: String) {
    providers.set_qwen_login_auth_url(login_id, auth_url).await;
}

pub(super) async fn set_completion_status(
    providers: &ProvidersHandle,
    login_id: &str,
    account_id: Option<String>,
    restart_result: anyhow::Result<()>,
) {
    providers
        .finish_qwen_login_session(login_id, account_id, restart_result)
        .await;
}

pub(super) async fn cleanup_login_home(login_home: &StdPath) {
    let _ = tokio::fs::remove_dir_all(login_home).await;
}
