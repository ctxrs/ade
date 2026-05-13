use super::*;

pub(super) async fn complete_qwen_login_if_credentials_exist(
    state: &Arc<AppState>,
    login_id: &str,
    label: &Option<String>,
    paths: &setup::QwenLoginPaths,
    observed_email: Option<String>,
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

    let added = crate::daemon::providers::add_qwen_account_for_login(
        state,
        label.clone(),
        oauth_raw,
        observed_email,
    )
    .await;
    match added {
        Ok(outcome) => {
            let (active_account_id, restart_result) = outcome.into_restart_result();
            status::set_completion_status(state, login_id, active_account_id, restart_result).await;
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

async fn read_trimmed_file(path: &StdPath) -> Option<String> {
    tokio::fs::read_to_string(path)
        .await
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
