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
    providers: ProvidersHandle,
    login_id: String,
    label: Option<String>,
) {
    let paths = match setup::prepare_mistral_login_paths(&providers, &login_id).await {
        Ok(paths) => paths,
        Err(error) => {
            status::set_failed(&providers, &login_id, error).await;
            return;
        }
    };
    let provider_env = setup::mistral_provider_env(&providers, &paths.mistral_home);

    let (event_tx, mut event_rx) = mpsc::channel(64);
    let auth_result = providers
        .authenticate_provider_session(
            "mistral",
            ctx_provider_runtime::provider_session_auth::ProviderSessionAuthenticationRequest {
                session_key: format!("mistral-login-{login_id}"),
                workdir: paths.workdir.clone(),
                env: provider_env,
                method_id: None,
                event_sink: event_tx,
                hooks: ctx_providers::adapters::ProviderRunHooks::default(),
            },
        )
        .await;
    match auth_result {
        Ok(()) => {}
        Err(ctx_provider_runtime::provider_session_auth::ProviderSessionAuthenticationError::AdapterUnavailable) => {
            status::set_failed(
                &providers,
                &login_id,
                "provider adapter not available".to_string(),
            )
            .await;
            status::cleanup_login_home(&paths.login_home).await;
            return;
        }
        Err(ctx_provider_runtime::provider_session_auth::ProviderSessionAuthenticationError::Authenticate(err)) => {
            status::set_failed(&providers, &login_id, logs::redact_sensitive(&err.to_string())).await;
            status::cleanup_login_home(&paths.login_home).await;
            return;
        }
    }

    let started_at = Instant::now();
    let timeout = mistral_login_timeout();
    let mut progress = events::MistralLoginProgress::default();

    loop {
        if started_at.elapsed() >= timeout {
            status::set_timeout_if_no_error(
                &providers,
                &login_id,
                "timed out waiting for Mistral OAuth completion".to_string(),
            )
            .await;
            status::cleanup_login_home(&paths.login_home).await;
            return;
        }

        match events::next_mistral_login_event(&providers, &login_id, &mut event_rx, &mut progress)
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
                status::set_failed_if_no_error(&providers, &login_id, message.to_string()).await;
                status::cleanup_login_home(&paths.login_home).await;
                return;
            }
            events::MistralLoginEventOutcome::Success => {
                completion::complete_mistral_login(
                    &providers,
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
