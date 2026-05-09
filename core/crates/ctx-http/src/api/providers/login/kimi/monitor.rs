use std::sync::Arc;
use std::time::{Duration, Instant};

use ctx_observability::logs;
use ctx_provider_accounts as provider_accounts;

use super::super::super::restarts;
use super::oauth;
use crate::daemon::AppState;

pub(super) async fn monitor_kimi_login(
    state: Arc<AppState>,
    login_id: String,
    label: Option<String>,
    device_code: String,
    poll_interval: Duration,
    timeout: Duration,
) {
    let started_at = Instant::now();

    loop {
        if started_at.elapsed() >= timeout {
            let mut map = state.providers.kimi_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "timeout".to_string();
                if entry.error.is_none() {
                    entry.error = Some("timed out waiting for Kimi sign-in completion".to_string());
                }
            }
            return;
        }

        match oauth::poll_kimi_token(&device_code).await {
            Ok(Ok(token)) => {
                let added = provider_accounts::add_kimi_oauth_account(
                    &state.core.data_root,
                    label.clone(),
                    oauth::kimi_token_json(&token),
                    None,
                )
                .await;
                match added {
                    Ok(registry) => {
                        let restart_result = restarts::restart_kimi_providers_for_auth_change(
                            &state,
                            "kimi auth updated",
                        )
                        .await;
                        let mut map = state.providers.kimi_login_sessions.lock().await;
                        if let Some(entry) = map.get_mut(&login_id) {
                            entry.account_id = registry.active_account_id.clone();
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
                    }
                    Err(err) => {
                        let mut map = state.providers.kimi_login_sessions.lock().await;
                        if let Some(entry) = map.get_mut(&login_id) {
                            entry.status = "failed".to_string();
                            entry.error = Some(logs::redact_sensitive(&err.to_string()));
                        }
                    }
                }
                return;
            }
            Ok(Err(error)) => {
                let error_code = error.error.as_deref().unwrap_or("oauth_error");
                if matches!(
                    error_code,
                    "authorization_pending" | "slow_down" | "access_denied"
                ) {
                    if error_code == "access_denied" {
                        let mut map = state.providers.kimi_login_sessions.lock().await;
                        if let Some(entry) = map.get_mut(&login_id) {
                            entry.status = "failed".to_string();
                            entry.error = Some(
                                error
                                    .error_description
                                    .unwrap_or_else(|| "Kimi sign-in was denied.".to_string()),
                            );
                        }
                        return;
                    }
                    tokio::time::sleep(poll_interval).await;
                    continue;
                }
                let mut map = state.providers.kimi_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = if error_code == "expired_token" {
                        "timeout".to_string()
                    } else {
                        "failed".to_string()
                    };
                    entry.error = Some(
                        error
                            .error_description
                            .unwrap_or_else(|| format!("Kimi sign-in failed: {error_code}")),
                    );
                }
                return;
            }
            Err(err) => {
                let mut map = state.providers.kimi_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "failed".to_string();
                    entry.error = Some(logs::redact_sensitive(&err.to_string()));
                }
                return;
            }
        }
    }
}
