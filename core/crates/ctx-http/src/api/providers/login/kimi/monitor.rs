use std::sync::Arc;
use std::time::{Duration, Instant};

use ctx_observability::logs;

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
            state
                .providers
                .with_kimi_login_sessions(|map| {
                    if let Some(entry) = map.get_mut(&login_id) {
                        entry.status = "timeout".to_string();
                        if entry.error.is_none() {
                            entry.error =
                                Some("timed out waiting for Kimi sign-in completion".to_string());
                        }
                    }
                })
                .await;
            return;
        }

        match oauth::poll_kimi_token(&device_code).await {
            Ok(Ok(token)) => {
                let added = crate::daemon::providers::add_kimi_oauth_account_for_login(
                    &state,
                    label.clone(),
                    oauth::kimi_token_json(&token),
                    None,
                )
                .await;
                match added {
                    Ok(outcome) => {
                        let restart_error = outcome.restart_error_message();
                        state
                            .providers
                            .with_kimi_login_sessions(|map| {
                                if let Some(entry) = map.get_mut(&login_id) {
                                    entry.account_id = outcome.active_account_id.clone();
                                    if let Some(error) = restart_error.as_deref() {
                                        entry.status = "failed".to_string();
                                        entry.error = Some(logs::redact_sensitive(error));
                                    } else {
                                        entry.status = "success".to_string();
                                        entry.error = None;
                                    }
                                }
                            })
                            .await;
                    }
                    Err(err) => {
                        state
                            .providers
                            .with_kimi_login_sessions(|map| {
                                if let Some(entry) = map.get_mut(&login_id) {
                                    entry.status = "failed".to_string();
                                    entry.error = Some(logs::redact_sensitive(
                                        &err.auth_login_error_message(),
                                    ));
                                }
                            })
                            .await;
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
                        state
                            .providers
                            .with_kimi_login_sessions(|map| {
                                if let Some(entry) = map.get_mut(&login_id) {
                                    entry.status = "failed".to_string();
                                    entry.error =
                                        Some(error.error_description.unwrap_or_else(|| {
                                            "Kimi sign-in was denied.".to_string()
                                        }));
                                }
                            })
                            .await;
                        return;
                    }
                    tokio::time::sleep(poll_interval).await;
                    continue;
                }
                state
                    .providers
                    .with_kimi_login_sessions(|map| {
                        if let Some(entry) = map.get_mut(&login_id) {
                            entry.status = if error_code == "expired_token" {
                                "timeout".to_string()
                            } else {
                                "failed".to_string()
                            };
                            entry.error =
                                Some(error.error_description.unwrap_or_else(|| {
                                    format!("Kimi sign-in failed: {error_code}")
                                }));
                        }
                    })
                    .await;
                return;
            }
            Err(err) => {
                state
                    .providers
                    .with_kimi_login_sessions(|map| {
                        if let Some(entry) = map.get_mut(&login_id) {
                            entry.status = "failed".to_string();
                            entry.error = Some(logs::redact_sensitive(&err.to_string()));
                        }
                    })
                    .await;
                return;
            }
        }
    }
}
