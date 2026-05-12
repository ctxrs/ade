use super::*;

#[path = "monitor/completion.rs"]
mod completion;
#[path = "monitor/events.rs"]
mod events;
#[path = "monitor/setup.rs"]
mod setup;
#[path = "monitor/status.rs"]
mod status;

pub(super) async fn monitor_qwen_login(
    state: Arc<AppState>,
    login_id: String,
    label: Option<String>,
) {
    let adapter = state.providers.provider_adapter("qwen").await;
    let Some(adapter) = adapter else {
        status::set_failed(
            &state,
            &login_id,
            "provider adapter not available".to_string(),
        )
        .await;
        return;
    };

    let paths = match setup::prepare_qwen_login_paths(&state, &login_id).await {
        Ok(paths) => paths,
        Err(error) => {
            status::set_failed(&state, &login_id, error).await;
            return;
        }
    };
    let provider_env = setup::qwen_provider_env(&state, &paths.login_home);

    let (event_tx, mut event_rx) = mpsc::channel(64);
    let auth_result = adapter
        .authenticate_session(
            format!("qwen-login-{login_id}"),
            paths.workdir.clone(),
            provider_env,
            Some(QWEN_OAUTH_AUTH_METHOD_ID.to_string()),
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
    let timeout = qwen_login_timeout();
    let mut progress = events::QwenLoginProgress::default();

    loop {
        let event_outcome =
            events::drain_qwen_login_events(&state, &login_id, &mut event_rx, &mut progress).await;
        if event_outcome.failed {
            status::cleanup_login_home(&paths.login_home).await;
            return;
        }

        if completion::complete_qwen_login_if_credentials_exist(
            &state,
            &login_id,
            &label,
            &paths,
            progress.observed_email.clone(),
        )
        .await
        {
            return;
        }

        if event_outcome.channel_disconnected && !progress.observed_auth_url {
            status::set_failed_if_no_error(
                &state,
                &login_id,
                "Qwen sign-in did not emit an OAuth URL in this environment.".to_string(),
            )
            .await;
            status::cleanup_login_home(&paths.login_home).await;
            return;
        }

        if started_at.elapsed() >= timeout {
            status::set_timeout_if_no_error(
                &state,
                &login_id,
                "timed out waiting for Qwen OAuth completion".to_string(),
            )
            .await;
            status::cleanup_login_home(&paths.login_home).await;
            return;
        }

        tokio::time::sleep(QWEN_LOGIN_POLL_INTERVAL).await;
    }
}
