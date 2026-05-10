use super::*;

pub(super) async fn complete_amp_login(
    state: &Arc<AppState>,
    login_id: &str,
    label: &Option<String>,
    observed_email: Option<String>,
) {
    if let Err(err) =
        provider_accounts::upsert_amp_account(&state.core.data_root, label.clone(), observed_email)
            .await
    {
        status::set_failed(state, login_id, logs::redact_sensitive(&err.to_string())).await;
        return;
    }
    let restart_result =
        restarts::restart_amp_providers_for_auth_change(state, "amp auth updated").await;
    status::set_completion_status(state, login_id, restart_result).await;
}
