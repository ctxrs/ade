use super::*;

pub(super) async fn set_failed(state: &Arc<AppState>, login_id: &str, error: String) {
    crate::daemon::providers::set_qwen_login_failed(state, login_id, error).await;
}

pub(super) async fn set_failed_if_no_error(state: &Arc<AppState>, login_id: &str, error: String) {
    crate::daemon::providers::set_qwen_login_failed_if_no_error(state, login_id, error).await;
}

pub(super) async fn set_timeout_if_no_error(state: &Arc<AppState>, login_id: &str, error: String) {
    crate::daemon::providers::set_qwen_login_timeout_if_no_error(state, login_id, error).await;
}

pub(super) async fn set_auth_url(state: &Arc<AppState>, login_id: &str, auth_url: String) {
    crate::daemon::providers::set_qwen_login_auth_url(state, login_id, auth_url).await;
}

pub(super) async fn set_completion_status(
    state: &Arc<AppState>,
    login_id: &str,
    account_id: Option<String>,
    restart_result: anyhow::Result<()>,
) {
    crate::daemon::providers::finish_qwen_login_session(
        state,
        login_id,
        account_id,
        restart_result,
    )
    .await;
}

pub(super) async fn cleanup_login_home(login_home: &StdPath) {
    let _ = tokio::fs::remove_dir_all(login_home).await;
}
