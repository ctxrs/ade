use super::*;

pub(super) async fn complete_mistral_login(
    providers: &ProvidersHandle,
    login_id: &str,
    label: &Option<String>,
    observed_email: Option<String>,
) {
    match providers
        .upsert_mistral_account_for_login(label.clone(), observed_email)
        .await
    {
        Ok(outcome) => {
            let (_, restart_result) = outcome.into_restart_result();
            status::set_completion_status(providers, login_id, restart_result).await;
        }
        Err(err) => {
            status::set_failed(
                providers,
                login_id,
                logs::redact_sensitive(&err.auth_login_error_message()),
            )
            .await;
        }
    }
}
