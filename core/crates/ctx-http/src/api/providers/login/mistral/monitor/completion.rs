use super::*;

pub(super) async fn complete_mistral_login(
    state: &Arc<AppState>,
    login_id: &str,
    label: &Option<String>,
    observed_email: Option<String>,
) {
    if let Err(err) = provider_accounts::upsert_mistral_account(
        &state.core.data_root,
        label.clone(),
        observed_email,
    )
    .await
    {
        status::set_failed(state, login_id, logs::redact_sensitive(&err.to_string())).await;
        return;
    }
    let restart_result = crate::daemon::providers::restart_mistral_providers_for_auth_change(
        state,
        "mistral auth updated",
    )
    .await;
    status::set_completion_status(state, login_id, restart_result).await;
}
