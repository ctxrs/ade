use super::*;

mod completion;
mod events;
mod setup;
mod status;

pub(super) async fn monitor_gemini_login(
    state: Arc<AppState>,
    login_id: String,
    label: Option<String>,
) {
    let paths = match setup::prepare_gemini_login_paths(&state, &login_id).await {
        Ok(paths) => paths,
        Err(err) => {
            status::set_failed(&state, &login_id, err).await;
            return;
        }
    };
    let provider_env = setup::gemini_provider_env(&state, &paths.login_home);

    let (event_tx, mut event_rx) = mpsc::channel(64);
    let auth_result = state
        .providers
        .authenticate_provider_session(
            "gemini",
            ctx_provider_runtime::provider_session_auth::ProviderSessionAuthenticationRequest {
                session_key: format!("gemini-login-{login_id}"),
                workdir: paths.workdir.clone(),
                env: provider_env,
                method_id: Some(crate::daemon::providers::gemini_login_auth_method_id()),
                event_sink: event_tx,
                hooks: ctx_providers::adapters::ProviderRunHooks::default(),
            },
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
    let timeout = gemini_login_timeout();
    let mut observed_auth_url = false;

    loop {
        let event_outcome =
            events::drain_gemini_login_events(&state, &login_id, &mut event_rx, observed_auth_url)
                .await;
        if event_outcome.failed {
            status::cleanup_login_home(&paths.login_home).await;
            return;
        }
        observed_auth_url = event_outcome.observed_auth_url;

        if completion::complete_gemini_login_if_credentials_exist(&state, &login_id, &label, &paths)
            .await
        {
            return;
        }

        if event_outcome.channel_disconnected && !observed_auth_url {
            status::set_failed_if_no_error(
                &state,
                &login_id,
                "Gemini sign-in did not emit an OAuth URL; the runtime may require API-key auth in this environment."
                    .to_string(),
            )
            .await;
            status::cleanup_login_home(&paths.login_home).await;
            return;
        }

        if started_at.elapsed() >= timeout {
            status::set_timeout_if_no_error(
                &state,
                &login_id,
                "timed out waiting for Gemini OAuth completion".to_string(),
            )
            .await;
            status::cleanup_login_home(&paths.login_home).await;
            return;
        }

        tokio::time::sleep(GEMINI_LOGIN_POLL_INTERVAL).await;
    }
}
