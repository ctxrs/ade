use super::*;

pub(super) async fn complete_amp_login(
    state: &Arc<AppState>,
    login_id: &str,
    label: &Option<String>,
    observed_email: Option<String>,
) {
    match crate::daemon::providers::upsert_amp_account_for_login(
        state,
        label.clone(),
        observed_email,
    )
    .await
    {
        Ok(outcome) => {
            let (_, restart_result) = outcome.into_restart_result();
            status::set_completion_status(state, login_id, restart_result).await;
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
}
