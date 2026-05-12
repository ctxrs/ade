use super::*;

pub(super) async fn set_failed(state: &Arc<AppState>, login_id: &str, error: String) {
    state
        .providers
        .with_amp_login_sessions(|map| {
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
        .with_amp_login_sessions(|map| {
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
        .with_amp_login_sessions(|map| {
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
        .with_amp_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.auth_url = Some(auth_url);
            }
        })
        .await;
}

pub(super) async fn set_completion_status(
    state: &Arc<AppState>,
    login_id: &str,
    restart_result: anyhow::Result<()>,
) {
    state
        .providers
        .with_amp_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.auth_url = None;
                match restart_result {
                    Ok(()) => {
                        entry.status = "success".to_string();
                        entry.error = None;
                    }
                    Err(err) => {
                        entry.status = "failed".to_string();
                        entry.error = Some(logs::redact_sensitive(&format!(
                            "auth saved but provider restart failed: {err:#}"
                        )));
                    }
                }
            }
        })
        .await;
}

pub(super) async fn cleanup_login_home(login_home: &StdPath) {
    let _ = tokio::fs::remove_dir_all(login_home).await;
}
