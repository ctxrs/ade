use std::path::Path;
use std::sync::Arc;

use crate::daemon::AppState;

pub(super) async fn set_failed(state: &Arc<AppState>, login_id: &str, error: String) {
    crate::daemon::providers::set_gemini_login_failed(state, login_id, error).await;
}

pub(super) async fn set_failed_if_no_error(state: &Arc<AppState>, login_id: &str, error: String) {
    crate::daemon::providers::set_gemini_login_failed_if_no_error(state, login_id, error).await;
}

pub(super) async fn set_timeout_if_no_error(state: &Arc<AppState>, login_id: &str, error: String) {
    crate::daemon::providers::set_gemini_login_timeout_if_no_error(state, login_id, error).await;
}

pub(super) async fn set_auth_url(state: &Arc<AppState>, login_id: &str, auth_url: String) {
    crate::daemon::providers::set_gemini_login_auth_url(state, login_id, auth_url).await;
}

pub(super) async fn cleanup_login_home(login_home: &Path) {
    let _ = tokio::fs::remove_dir_all(login_home).await;
}
