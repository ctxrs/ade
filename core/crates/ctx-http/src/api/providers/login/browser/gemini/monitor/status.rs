use std::path::Path;

use ctx_daemon::daemon::ProvidersHandle;

pub(super) async fn set_failed(providers: &ProvidersHandle, login_id: &str, error: String) {
    providers.set_gemini_login_failed(login_id, error).await;
}

pub(super) async fn set_failed_if_no_error(
    providers: &ProvidersHandle,
    login_id: &str,
    error: String,
) {
    providers
        .set_gemini_login_failed_if_no_error(login_id, error)
        .await;
}

pub(super) async fn set_timeout_if_no_error(
    providers: &ProvidersHandle,
    login_id: &str,
    error: String,
) {
    providers
        .set_gemini_login_timeout_if_no_error(login_id, error)
        .await;
}

pub(super) async fn set_auth_url(providers: &ProvidersHandle, login_id: &str, auth_url: String) {
    providers
        .set_gemini_login_auth_url(login_id, auth_url)
        .await;
}

pub(super) async fn cleanup_login_home(login_home: &Path) {
    let _ = tokio::fs::remove_dir_all(login_home).await;
}
