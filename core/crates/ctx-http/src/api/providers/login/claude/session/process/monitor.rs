use super::line_observation::append_claude_login_line;
use super::*;

#[path = "monitor/finalization.rs"]
mod finalization;

use finalization::finalize_claude_login;

pub(in crate::api::providers::login::claude::session) async fn kill_claude_login_process(
    killer: Arc<StdMutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || {
        let mut guard = killer
            .lock()
            .map_err(|_| anyhow::anyhow!("claude setup-token killer lock poisoned"))?;
        guard.kill().context("killing claude setup-token process")
    })
    .await
    .context("joining claude setup-token kill task")?
}

pub(in crate::api::providers::login::claude::session) async fn monitor_claude_login(
    state: Arc<AppState>,
    login_id: String,
    label: Option<String>,
    mut login: ClaudeLoginProcess,
) {
    let mut transcript = String::new();
    let mut observed_auth_url = login.auth_url.clone();
    let mut output_closed = false;
    let auth_url_deadline = Instant::now() + CLAUDE_LOGIN_NO_AUTH_URL_TIMEOUT;
    let mut completion_deadline = observed_auth_url
        .as_ref()
        .map(|_| Instant::now() + CLAUDE_LOGIN_COMPLETION_TIMEOUT);
    let mut exit_result: Option<anyhow::Result<portable_pty::ExitStatus>> = None;
    let mut terminal_error: Option<String> = None;

    for line in std::mem::take(&mut login.buffered_lines) {
        let had_auth_url = observed_auth_url.is_some();
        append_claude_login_line(
            &state,
            &login_id,
            &mut observed_auth_url,
            &mut transcript,
            line,
        )
        .await;
        let _ = refresh_claude_auth_url_from_capture_path(
            &mut observed_auth_url,
            &login.browser_open_capture_path,
        );
        if claude_manual_fallback_is_terminal(&transcript, &login.browser_open_capture_path) {
            terminal_error = Some(CLAUDE_UNSUPPORTED_MANUAL_FALLBACK_ERROR.to_string());
            break;
        }
        if !had_auth_url && observed_auth_url.is_some() {
            completion_deadline = Some(Instant::now() + CLAUDE_LOGIN_COMPLETION_TIMEOUT);
        }
    }

    while terminal_error.is_none() {
        let _ = refresh_claude_auth_url_from_capture_path(
            &mut observed_auth_url,
            &login.browser_open_capture_path,
        );
        let deadline = completion_deadline.unwrap_or(auth_url_deadline);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            terminal_error = Some(if observed_auth_url.is_some() {
                "claude setup-token timed out waiting for browser sign-in completion".to_string()
            } else {
                "claude setup-token did not emit an authentication URL".to_string()
            });
            break;
        }
        let timeout_future = tokio::time::sleep(remaining);
        tokio::pin!(timeout_future);

        tokio::select! {
            maybe_line = login.line_rx.recv(), if !output_closed => {
                match maybe_line {
                    Some(line) => {
                        let had_auth_url = observed_auth_url.is_some();
                        append_claude_login_line(
                            &state,
                            &login_id,
                            &mut observed_auth_url,
                            &mut transcript,
                            line,
                        )
                        .await;
                        let _ = refresh_claude_auth_url_from_capture_path(
                            &mut observed_auth_url,
                            &login.browser_open_capture_path,
                        );
                        if claude_manual_fallback_is_terminal(
                            &transcript,
                            &login.browser_open_capture_path,
                        ) {
                            terminal_error = Some(CLAUDE_UNSUPPORTED_MANUAL_FALLBACK_ERROR.to_string());
                            break;
                        }
                        if !had_auth_url && observed_auth_url.is_some() {
                            completion_deadline = Some(Instant::now() + CLAUDE_LOGIN_COMPLETION_TIMEOUT);
                        }
                    }
                    None => {
                        output_closed = true;
                    }
                }
            }
            exit = &mut login.exit_rx => {
                exit_result = Some(match exit {
                    Ok(result) => result,
                    Err(err) => Err(anyhow::anyhow!("claude setup-token exit channel closed: {err}")),
                });
                break;
            }
            _ = &mut timeout_future => {
                terminal_error = Some(if observed_auth_url.is_some() {
                    "claude setup-token timed out waiting for browser sign-in completion".to_string()
                } else {
                    "claude setup-token did not emit an authentication URL".to_string()
                });
                break;
            }
        }
    }

    if exit_result.is_some() {
        for line in
            read_trailing_claude_login_lines(&mut login.line_rx, CLAUDE_LOGIN_EXIT_GRACE_WAIT).await
        {
            append_claude_login_line(
                &state,
                &login_id,
                &mut observed_auth_url,
                &mut transcript,
                line,
            )
            .await;
            let _ = refresh_claude_auth_url_from_capture_path(
                &mut observed_auth_url,
                &login.browser_open_capture_path,
            );
        }
    } else {
        while let Ok(line) = login.line_rx.try_recv() {
            append_claude_login_line(
                &state,
                &login_id,
                &mut observed_auth_url,
                &mut transcript,
                line,
            )
            .await;
            let _ = refresh_claude_auth_url_from_capture_path(
                &mut observed_auth_url,
                &login.browser_open_capture_path,
            );
        }
    }

    if terminal_error.is_some() {
        if let Err(err) = kill_claude_login_process(Arc::clone(&login.killer)).await {
            let suffix = format!("; failed to terminate setup-token process cleanly: {err}");
            terminal_error = Some(match terminal_error.take() {
                Some(base) => format!("{base}{suffix}"),
                None => suffix,
            });
        }
        if exit_result.is_none() {
            if let Ok(exit) =
                tokio::time::timeout(CLAUDE_LOGIN_EXIT_GRACE_WAIT, &mut login.exit_rx).await
            {
                exit_result = Some(match exit {
                    Ok(result) => result,
                    Err(err) => Err(anyhow::anyhow!(
                        "claude setup-token exit channel closed: {err}"
                    )),
                });
            }
        }
    }

    finalize_claude_login(
        &state,
        &login_id,
        label,
        observed_auth_url,
        terminal_error,
        exit_result,
        &transcript,
    )
    .await;
}
