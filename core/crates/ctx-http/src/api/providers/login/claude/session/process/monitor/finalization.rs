use super::*;

fn format_claude_exit_status(status: &portable_pty::ExitStatus) -> String {
    if let Some(signal) = status.signal() {
        return format!("signal {signal}");
    }
    status.exit_code().to_string()
}

pub(super) async fn finalize_claude_login(
    state: &Arc<AppState>,
    login_id: &str,
    label: Option<String>,
    observed_auth_url: Option<String>,
    terminal_error: Option<String>,
    exit_result: Option<anyhow::Result<portable_pty::ExitStatus>>,
    transcript: &str,
) {
    let mut final_status = "failed".to_string();
    let mut final_error = terminal_error;
    let mut final_account_id: Option<String> = None;

    if final_error.is_none() {
        match exit_result {
            Some(Ok(exit)) if exit.success() => match extract_claude_setup_token(transcript) {
                Some(setup_token) => {
                    match provider_accounts::add_claude_account(
                        &state.core.data_root,
                        label,
                        setup_token,
                    )
                    .await
                    {
                        Ok(registry) => {
                            final_account_id = registry.active_account_id;
                            match restarts::restart_claude_providers_for_auth_change(
                                state,
                                "claude auth updated",
                            )
                            .await
                            {
                                Ok(()) => {
                                    final_status = "success".to_string();
                                }
                                Err(err) => {
                                    final_error = Some(logs::redact_sensitive(&format!(
                                        "auth saved but provider restart failed: {err:#}"
                                    )));
                                }
                            }
                        }
                        Err(err) => {
                            final_error = Some(logs::redact_sensitive(&err.to_string()));
                        }
                    }
                }
                None => {
                    final_error = Some(
                        "claude setup-token completed but no setup token was detected".to_string(),
                    );
                }
            },
            Some(Ok(exit)) => {
                final_error = Some(format!(
                    "claude setup-token exited with status {}",
                    format_claude_exit_status(&exit)
                ));
            }
            Some(Err(err)) => {
                final_error = Some(format!("waiting for claude setup-token failed: {err}"));
            }
            None => {
                final_error = Some(
                    "claude setup-token monitor ended before process exit was observed".to_string(),
                );
            }
        }
    }

    state
        .providers
        .with_claude_login_sessions(|map| {
            if let Some(entry) = map.get_mut(login_id) {
                entry.status = final_status;
                entry.account_id = final_account_id;
                entry.error = final_error;
                if entry.auth_url.is_none() {
                    entry.auth_url = observed_auth_url;
                }
            }
        })
        .await;
}
