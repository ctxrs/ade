use super::*;

#[path = "monitor/completion.rs"]
mod completion;
#[path = "monitor/events.rs"]
mod events;
#[path = "monitor/setup.rs"]
mod setup;
#[path = "monitor/status.rs"]
mod status;

pub(super) async fn monitor_mistral_login(
    state: Arc<AppState>,
    login_id: String,
    label: Option<String>,
) {
    let adapter = state.providers.provider_adapter("mistral").await;
    let Some(adapter) = adapter else {
        status::set_failed(
            &state,
            &login_id,
            "provider adapter not available".to_string(),
        )
        .await;
        return;
    };

    let paths = match setup::prepare_mistral_login_paths(&state, &login_id).await {
        Ok(paths) => paths,
        Err(error) => {
            status::set_failed(&state, &login_id, error).await;
            return;
        }
    };
    let provider_env = setup::mistral_provider_env(&state, &paths.mistral_home);

    let (event_tx, mut event_rx) = mpsc::channel(64);
    let auth_result = adapter
        .authenticate_session(
            format!("mistral-login-{login_id}"),
            paths.workdir.clone(),
            provider_env,
            None,
            event_tx,
            ctx_providers::adapters::ProviderRunHooks::default(),
        )
        .await;
    if let Err(err) = auth_result {
        status::set_failed(&state, &login_id, logs::redact_sensitive(&err.to_string())).await;
        status::cleanup_login_home(&paths.login_home).await;
        return;
    }

    let started_at = Instant::now();
    let timeout = mistral_login_timeout();
    let mut progress = events::MistralLoginProgress::default();

    loop {
        if started_at.elapsed() >= timeout {
            status::set_timeout_if_no_error(
                &state,
                &login_id,
                "timed out waiting for Mistral OAuth completion".to_string(),
            )
            .await;
            status::cleanup_login_home(&paths.login_home).await;
            return;
        }

        match events::next_mistral_login_event(&state, &login_id, &mut event_rx, &mut progress)
            .await
        {
            events::MistralLoginEventOutcome::Pending => {}
            events::MistralLoginEventOutcome::Failed => {
                status::cleanup_login_home(&paths.login_home).await;
                return;
            }
            events::MistralLoginEventOutcome::ChannelDisconnected => {
                let message = if progress.observed_auth_url {
                    "Mistral sign-in session ended before completion."
                } else {
                    "Mistral sign-in did not emit an OAuth URL in this environment."
                };
                status::set_failed_if_no_error(&state, &login_id, message.to_string()).await;
                status::cleanup_login_home(&paths.login_home).await;
                return;
            }
            events::MistralLoginEventOutcome::Success => {
                completion::complete_mistral_login(
                    &state,
                    &login_id,
                    &label,
                    progress.observed_email.clone(),
                )
                .await;
                status::cleanup_login_home(&paths.login_home).await;
                return;
            }
        }
    }
}
