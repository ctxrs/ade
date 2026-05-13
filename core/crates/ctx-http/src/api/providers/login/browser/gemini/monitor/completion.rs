use std::sync::Arc;

use super::setup::GeminiLoginPaths;
use super::status;
use crate::api::providers::login::first_email_from_google_accounts;
use crate::daemon::AppState;
use ctx_observability::logs;

pub(super) async fn complete_gemini_login_if_credentials_exist(
    state: &Arc<AppState>,
    login_id: &str,
    label: &Option<String>,
    paths: &GeminiLoginPaths,
) -> bool {
    let Some(oauth_raw) = read_trimmed_file(&paths.oauth_path).await else {
        return false;
    };
    let oauth_value = serde_json::from_str::<serde_json::Value>(&oauth_raw);
    let oauth_valid = oauth_value
        .as_ref()
        .ok()
        .is_some_and(serde_json::Value::is_object);
    if !oauth_valid {
        status::set_failed(
            state,
            login_id,
            "captured oauth_creds.json is not a valid JSON object".to_string(),
        )
        .await;
        status::cleanup_login_home(&paths.login_home).await;
        return true;
    }

    let google_accounts_raw = read_trimmed_file(&paths.google_accounts_path).await;
    let google_accounts_value = google_accounts_raw
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
    let email = google_accounts_value
        .as_ref()
        .and_then(first_email_from_google_accounts);
    let added = crate::daemon::providers::add_gemini_account_for_login(
        state,
        label.clone(),
        oauth_raw,
        google_accounts_raw,
        email,
    )
    .await;
    match added {
        Ok(outcome) => {
            let restart_error = outcome.restart_error_message();
            state
                .providers
                .with_gemini_login_sessions(|map| {
                    if let Some(entry) = map.get_mut(login_id) {
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
            status::set_failed(
                state,
                login_id,
                logs::redact_sensitive(&err.auth_login_error_message()),
            )
            .await;
        }
    }
    status::cleanup_login_home(&paths.login_home).await;
    true
}

async fn read_trimmed_file(path: &std::path::Path) -> Option<String> {
    tokio::fs::read_to_string(path)
        .await
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
