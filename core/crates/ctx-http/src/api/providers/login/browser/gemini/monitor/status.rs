use std::path::Path;
use std::sync::Arc;

use crate::daemon::AppState;

pub(super) async fn set_failed(state: &Arc<AppState>, login_id: &str, error: String) {
    state
        .providers
        .with_gemini_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.status = "failed".to_string();
                entry.error = Some(error);
            }
        })
        .await;
}

pub(super) async fn set_failed_if_no_error(state: &Arc<AppState>, login_id: &str, error: String) {
    state
        .providers
        .with_gemini_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.status = "failed".to_string();
                if entry.error.is_none() {
                    entry.error = Some(error);
                }
            }
        })
        .await;
}

pub(super) async fn set_timeout_if_no_error(state: &Arc<AppState>, login_id: &str, error: String) {
    state
        .providers
        .with_gemini_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.status = "timeout".to_string();
                if entry.error.is_none() {
                    entry.error = Some(error);
                }
            }
        })
        .await;
}

pub(super) async fn set_auth_url(state: &Arc<AppState>, login_id: &str, auth_url: String) {
    state
        .providers
        .with_gemini_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.auth_url = Some(auth_url);
            }
        })
        .await;
}

pub(super) async fn cleanup_login_home(login_home: &Path) {
    let _ = tokio::fs::remove_dir_all(login_home).await;
}
