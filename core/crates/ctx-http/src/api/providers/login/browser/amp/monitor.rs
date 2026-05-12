use super::*;

#[path = "monitor/completion.rs"]
mod completion;
#[path = "monitor/events.rs"]
mod events;
#[path = "monitor/setup.rs"]
mod setup;
#[path = "monitor/status.rs"]
mod status;

pub(super) async fn monitor_amp_login(
    state: Arc<AppState>,
    login_id: String,
    label: Option<String>,
) {
    let paths = match setup::prepare_amp_login_paths(&state, &login_id).await {
        Ok(paths) => paths,
        Err(error) => {
            status::set_failed(&state, &login_id, error).await;
            return;
        }
    };
    let provider_env = setup::amp_provider_env(&state, &paths.amp_home);

    let (event_tx, mut event_rx) = mpsc::channel(64);
    let auth_result = state
        .providers
        .authenticate_provider_session(
            "amp",
            format!("amp-login-{login_id}"),
            paths.workdir.clone(),
            provider_env,
            Some(AMP_BROWSER_AUTH_METHOD_ID.to_string()),
            event_tx,
            ctx_providers::adapters::ProviderRunHooks::default(),
        )
        .await;
    match auth_result {
        Ok(()) => {}
        Err(ctx_provider_runtime::provider_session_auth::ProviderSessionAuthenticationError::AdapterUnavailable) => {
            status::set_failed(
                &state,
                &login_id,
                "provider adapter not available".to_string(),
            )
            .await;
            status::cleanup_login_home(&paths.login_home).await;
            return;
        }
        Err(ctx_provider_runtime::provider_session_auth::ProviderSessionAuthenticationError::Authenticate(err)) => {
            status::set_failed(&state, &login_id, logs::redact_sensitive(&err.to_string())).await;
            status::cleanup_login_home(&paths.login_home).await;
            return;
        }
    }

    let started_at = Instant::now();
    let timeout = amp_login_timeout();
    let mut progress = events::AmpLoginProgress::default();

    loop {
        if started_at.elapsed() >= timeout {
            status::set_timeout_if_no_error(
                &state,
                &login_id,
                "timed out waiting for Amp OAuth completion".to_string(),
            )
            .await;
            status::cleanup_login_home(&paths.login_home).await;
            return;
        }

        match events::next_amp_login_event(&state, &login_id, &mut event_rx, &mut progress).await {
            events::AmpLoginEventOutcome::Pending => {}
            events::AmpLoginEventOutcome::Failed => {
                status::cleanup_login_home(&paths.login_home).await;
                return;
            }
            events::AmpLoginEventOutcome::ChannelDisconnected => {
                let message = if progress.observed_auth_url {
                    "Amp sign-in session ended before completion."
                } else {
                    "Amp sign-in did not emit an OAuth URL in this environment."
                };
                status::set_failed_if_no_error(&state, &login_id, message.to_string()).await;
                status::cleanup_login_home(&paths.login_home).await;
                return;
            }
            events::AmpLoginEventOutcome::Success => {
                completion::complete_amp_login(
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
