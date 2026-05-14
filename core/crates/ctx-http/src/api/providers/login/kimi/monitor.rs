use std::time::{Duration, Instant};

use ctx_observability::logs;

use super::oauth;
use ctx_daemon::daemon::ProvidersHandle;

pub(super) async fn monitor_kimi_login(
    providers: ProvidersHandle,
    login_id: String,
    label: Option<String>,
    device_code: String,
    poll_interval: Duration,
    timeout: Duration,
) {
    let started_at = Instant::now();

    loop {
        if started_at.elapsed() >= timeout {
            providers
                .set_kimi_login_timeout_if_no_error(
                    &login_id,
                    "timed out waiting for Kimi sign-in completion".to_string(),
                )
                .await;
            return;
        }

        match oauth::poll_kimi_token(&device_code).await {
            Ok(Ok(token)) => {
                let added = providers
                    .add_kimi_oauth_account_for_login(
                        label.clone(),
                        oauth::kimi_token_json(&token),
                        None,
                    )
                    .await;
                match added {
                    Ok(outcome) => {
                        let restart_error = outcome.restart_error_message();
                        providers
                            .finish_kimi_login_session(
                                &login_id,
                                outcome.active_account_id,
                                restart_error,
                            )
                            .await;
                    }
                    Err(err) => {
                        providers
                            .set_kimi_login_failed(
                                &login_id,
                                logs::redact_sensitive(&err.auth_login_error_message()),
                            )
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
                        providers
                            .set_kimi_login_failed(
                                &login_id,
                                error
                                    .error_description
                                    .unwrap_or_else(|| "Kimi sign-in was denied.".to_string()),
                            )
                            .await;
                        return;
                    }
                    tokio::time::sleep(poll_interval).await;
                    continue;
                }
                let status = if error_code == "expired_token" {
                    "timeout"
                } else {
                    "failed"
                };
                providers
                    .set_kimi_login_terminal_status(
                        &login_id,
                        status,
                        error
                            .error_description
                            .unwrap_or_else(|| format!("Kimi sign-in failed: {error_code}")),
                    )
                    .await;
                return;
            }
            Err(err) => {
                providers
                    .set_kimi_login_failed(&login_id, logs::redact_sensitive(&err.to_string()))
                    .await;
                return;
            }
        }
    }
}
