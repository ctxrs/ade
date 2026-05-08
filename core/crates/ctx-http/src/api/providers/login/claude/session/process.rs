use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use anyhow::Context;
use tokio::sync::{mpsc, oneshot};

use super::super::super::{
    auth_url_looks_complete, read_trailing_claude_login_lines, CLAUDE_LOGIN_URL_WAIT,
};
use super::super::auth_url::{
    claude_login_hit_unsupported_manual_fallback, claude_manual_fallback_is_terminal,
    extract_claude_setup_token, extract_preferred_claude_auth_url,
    refresh_claude_auth_url_from_capture_path, should_replace_observed_claude_auth_url,
    CLAUDE_UNSUPPORTED_MANUAL_FALLBACK_ERROR,
};
use super::super::runtime::{
    resolve_claude_login_runtime, spawn_claude_setup_token_command, ClaudeLoginSpawn,
};
use super::*;

const CLAUDE_LOGIN_NO_AUTH_URL_TIMEOUT: Duration = Duration::from_secs(8);
const CLAUDE_LOGIN_URL_SETTLE_WAIT: Duration = Duration::from_millis(500);
const CLAUDE_LOGIN_CAPTURE_POLL_INTERVAL: Duration = Duration::from_millis(50);
const CLAUDE_LOGIN_COMPLETION_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const CLAUDE_LOGIN_EXIT_GRACE_WAIT: Duration = Duration::from_millis(400);

pub(super) struct ClaudeLoginProcess {
    line_rx: mpsc::UnboundedReceiver<String>,
    buffered_lines: Vec<String>,
    pub(super) auth_url: Option<String>,
    browser_open_capture_path: PathBuf,
    exit_rx: oneshot::Receiver<anyhow::Result<portable_pty::ExitStatus>>,
    killer: Arc<StdMutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    _browser_open_shim_dir: tempfile::TempDir,
}

pub(super) async fn start_claude_login_process(
    state: &Arc<AppState>,
) -> anyhow::Result<ClaudeLoginProcess> {
    let runtime = resolve_claude_login_runtime(state).await?;
    let ClaudeLoginSpawn {
        line_rx: mut rx,
        mut exit_rx,
        killer,
        browser_open_capture_path,
        browser_open_shim_dir,
    } = spawn_claude_setup_token_command(&runtime)?;

    let mut buffered_lines = Vec::new();
    let mut auth_url = None;
    let mut hit_unsupported_manual_fallback = false;
    let mut transcript = String::new();
    let hard_deadline = Instant::now() + CLAUDE_LOGIN_URL_WAIT;
    let mut settle_deadline: Option<Instant> = None;
    let mut output_closed = false;
    loop {
        if refresh_claude_auth_url_from_capture_path(&mut auth_url, &browser_open_capture_path) {
            settle_deadline = Some(Instant::now() + CLAUDE_LOGIN_URL_SETTLE_WAIT);
        }
        let now = Instant::now();
        let remaining = if let Some(settle) = settle_deadline {
            std::cmp::min(
                hard_deadline.saturating_duration_since(now),
                settle.saturating_duration_since(now),
            )
        } else {
            hard_deadline.saturating_duration_since(now)
        };
        if remaining.is_zero() {
            break;
        }
        let wait = remaining.min(CLAUDE_LOGIN_CAPTURE_POLL_INTERVAL);
        match tokio::time::timeout(wait, rx.recv()).await {
            Ok(Some(line)) => {
                output_closed = false;
                transcript.push_str(&line);
                transcript.push('\n');
                hit_unsupported_manual_fallback |=
                    claude_login_hit_unsupported_manual_fallback(&line);
                buffered_lines.push(line);
                if refresh_claude_auth_url_from_capture_path(
                    &mut auth_url,
                    &browser_open_capture_path,
                ) {
                    settle_deadline = Some(Instant::now() + CLAUDE_LOGIN_URL_SETTLE_WAIT);
                }
                if let Some((candidate, source)) = extract_preferred_claude_auth_url(&transcript) {
                    if should_replace_observed_claude_auth_url(
                        auth_url.as_deref(),
                        &candidate,
                        source,
                    ) {
                        auth_url = Some(candidate);
                    }
                    settle_deadline = Some(Instant::now() + CLAUDE_LOGIN_URL_SETTLE_WAIT);
                }
            }
            Ok(None) => {
                if !output_closed {
                    // The setup-token process can tear down its PTY before the
                    // browser shim capture file becomes visible on disk. Keep
                    // polling for one short settle window instead of exiting
                    // immediately on EOF.
                    output_closed = true;
                    settle_deadline = Some(Instant::now() + CLAUDE_LOGIN_URL_SETTLE_WAIT);
                }
                tokio::time::sleep(wait.min(CLAUDE_LOGIN_CAPTURE_POLL_INTERVAL)).await;
                continue;
            }
            Err(_) => continue,
        }
    }
    refresh_claude_auth_url_from_capture_path(&mut auth_url, &browser_open_capture_path);

    if hit_unsupported_manual_fallback
        && claude_manual_fallback_is_terminal(&transcript, &browser_open_capture_path)
    {
        let _ = kill_claude_login_process(Arc::clone(&killer)).await;
        let _ = tokio::time::timeout(CLAUDE_LOGIN_EXIT_GRACE_WAIT, &mut exit_rx).await;
        anyhow::bail!(CLAUDE_UNSUPPORTED_MANUAL_FALLBACK_ERROR);
    }

    Ok(ClaudeLoginProcess {
        line_rx: rx,
        buffered_lines,
        auth_url,
        browser_open_capture_path,
        exit_rx,
        killer,
        _browser_open_shim_dir: browser_open_shim_dir,
    })
}

async fn append_claude_login_line(
    state: &Arc<AppState>,
    login_id: &str,
    observed_auth_url: &mut Option<String>,
    transcript: &mut String,
    line: String,
) {
    let needs_auth_url_upgrade = match observed_auth_url.as_deref() {
        None => true,
        Some(url) => !auth_url_looks_complete(url),
    };
    if needs_auth_url_upgrade {
        if let Some((candidate, source)) = extract_preferred_claude_auth_url(&line) {
            if should_replace_observed_claude_auth_url(
                observed_auth_url.as_deref(),
                &candidate,
                source,
            ) {
                *observed_auth_url = Some(candidate);
            }
        }
    }
    transcript.push_str(&line);
    transcript.push('\n');
    let needs_auth_url_upgrade = match observed_auth_url.as_deref() {
        None => true,
        Some(url) => !auth_url_looks_complete(url),
    };
    if needs_auth_url_upgrade {
        if let Some((candidate, source)) = extract_preferred_claude_auth_url(transcript) {
            if should_replace_observed_claude_auth_url(
                observed_auth_url.as_deref(),
                &candidate,
                source,
            ) {
                *observed_auth_url = Some(candidate);
            }
        }
    }
    if let Some(url) = observed_auth_url.clone() {
        let mut map = state.providers.claude_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(login_id) {
            entry.auth_url = Some(url);
        }
    }
}

fn format_claude_exit_status(status: &portable_pty::ExitStatus) -> String {
    if let Some(signal) = status.signal() {
        return format!("signal {signal}");
    }
    status.exit_code().to_string()
}

async fn kill_claude_login_process(
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

pub(super) async fn monitor_claude_login(
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

    let mut final_status = "failed".to_string();
    let mut final_error: Option<String> = terminal_error;
    let mut final_account_id: Option<String> = None;

    if final_error.is_none() {
        match exit_result {
            Some(Ok(exit)) if exit.success() => match extract_claude_setup_token(&transcript) {
                Some(setup_token) => {
                    match provider_accounts::add_claude_account(
                        &state.core.data_root,
                        label.clone(),
                        setup_token,
                    )
                    .await
                    {
                        Ok(registry) => {
                            final_account_id = registry.active_account_id;
                            match restarts::restart_claude_providers_for_auth_change(
                                &state,
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

    {
        let mut map = state.providers.claude_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = final_status;
            entry.account_id = final_account_id;
            entry.error = final_error;
            if entry.auth_url.is_none() {
                entry.auth_url = observed_auth_url;
            }
        }
    }
}
