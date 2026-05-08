use super::super::login::extract_auth_url;
use super::capture::{
    cursor_login_home, ensure_private_dir, initialize_cursor_capture_file,
    parse_cursor_captured_tokens, write_cursor_capture_hook,
};
use super::output::{
    cursor_login_timeout, first_email_from_text, spawn_cursor_login_reader, CursorLoginOutputLine,
    CURSOR_LOGIN_POLL_INTERVAL,
};
use super::runtime::resolve_cursor_login_runtime;
use super::*;

async fn set_cursor_login_error(state: &Arc<AppState>, login_id: &str, error: String) {
    let mut map = state.providers.cursor_login_sessions.lock().await;
    if let Some(entry) = map.get_mut(login_id) {
        entry.status = "failed".to_string();
        entry.error = Some(error);
    }
}

async fn update_cursor_auth_url(state: &Arc<AppState>, login_id: &str, auth_url: String) {
    let mut map = state.providers.cursor_login_sessions.lock().await;
    if let Some(entry) = map.get_mut(login_id) {
        entry.auth_url = Some(auth_url);
    }
}

async fn record_cursor_login_output(
    state: &Arc<AppState>,
    login_id: &str,
    output_line: CursorLoginOutputLine,
    transcript: &mut String,
    observed_email: &mut Option<String>,
    observed_auth_url: &mut Option<String>,
) {
    transcript.push_str(&output_line.line);
    transcript.push('\n');
    if !output_line.is_stderr && observed_email.is_none() {
        *observed_email = first_email_from_text(&output_line.line);
    }
    if let Some(candidate) =
        extract_auth_url(&output_line.line).or_else(|| extract_auth_url(transcript))
    {
        let needs_update = observed_auth_url
            .as_ref()
            .is_none_or(|current| candidate.len() > current.len());
        if needs_update {
            *observed_auth_url = Some(candidate.clone());
            update_cursor_auth_url(state, login_id, candidate).await;
        }
    }
}

pub(super) async fn monitor_cursor_login(
    state: Arc<AppState>,
    login_id: String,
    label: Option<String>,
) {
    let cursor_runtime = match resolve_cursor_login_runtime(&state).await {
        Ok(runtime) => runtime,
        Err(err) => {
            set_cursor_login_error(&state, &login_id, logs::redact_sensitive(&err.to_string()))
                .await;
            return;
        }
    };

    let login_home = cursor_login_home(&state.core.data_root, &login_id);
    let workdir = login_home.join("workspace");
    let hook_path = login_home.join("capture-hook.cjs");
    let capture_path = login_home.join("captured_tokens.jsonl");

    if let Err(err) = async {
        ensure_private_dir(&login_home).await?;
        ensure_private_dir(&workdir).await
    }
    .await
    {
        set_cursor_login_error(
            &state,
            &login_id,
            format!("failed to prepare login workspace: {err}"),
        )
        .await;
        let _ = tokio::fs::remove_dir_all(&login_home).await;
        return;
    }

    if let Err(err) = write_cursor_capture_hook(&hook_path).await {
        set_cursor_login_error(&state, &login_id, logs::redact_sensitive(&err.to_string())).await;
        let _ = tokio::fs::remove_dir_all(&login_home).await;
        return;
    }
    if let Err(err) = initialize_cursor_capture_file(&capture_path).await {
        set_cursor_login_error(
            &state,
            &login_id,
            format!("failed to initialize capture file: {err}"),
        )
        .await;
        let _ = tokio::fs::remove_dir_all(&login_home).await;
        return;
    }

    let hook_require = format!("--require {}", hook_path.to_string_lossy());
    let node_options = match std::env::var("NODE_OPTIONS") {
        Ok(existing) if !existing.trim().is_empty() => {
            format!("{} {}", existing.trim(), hook_require)
        }
        _ => hook_require,
    };

    let mut cmd = Command::new(&cursor_runtime.command_abs_path);
    for key in ctx_core::env::DAEMON_AUTH_ENV_VARS {
        cmd.env_remove(key);
    }
    for arg in &cursor_runtime.args {
        cmd.arg(arg);
    }
    cmd.arg("login");
    cmd.current_dir(&workdir);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.env("NO_OPEN_BROWSER", "1");
    cmd.env(
        "CTX_CURSOR_CAPTURE_FILE",
        capture_path.to_string_lossy().to_string(),
    );
    cmd.env("NODE_OPTIONS", node_options);

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            set_cursor_login_error(
                &state,
                &login_id,
                format!("failed to launch cursor-agent login: {err}"),
            )
            .await;
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }
    };

    let mut transcript = String::new();
    let mut observed_auth_url = None::<String>;
    let mut observed_email = None::<String>;
    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<CursorLoginOutputLine>();
    if let Some(stdout) = child.stdout.take() {
        spawn_cursor_login_reader(stdout, false, line_tx.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_cursor_login_reader(stderr, true, line_tx.clone());
    }
    drop(line_tx);

    let started_at = Instant::now();
    let timeout = cursor_login_timeout();
    let mut timeout_error: Option<String> = None;
    let exit_result: std::io::Result<std::process::ExitStatus> = loop {
        if started_at.elapsed() >= timeout {
            timeout_error = Some("timed out waiting for Cursor OAuth completion".to_string());
            let _ = child.kill().await;
            break child.wait().await;
        }

        tokio::select! {
            maybe_line = line_rx.recv() => {
                if let Some(output_line) = maybe_line {
                    record_cursor_login_output(
                        &state,
                        &login_id,
                        output_line,
                        &mut transcript,
                        &mut observed_email,
                        &mut observed_auth_url,
                    )
                    .await;
                }
            }
            wait = child.wait() => {
                break wait;
            }
            _ = tokio::time::sleep(CURSOR_LOGIN_POLL_INTERVAL) => {}
        }
    };

    let drain_deadline = Instant::now() + std::time::Duration::from_millis(200);
    loop {
        match tokio::time::timeout(std::time::Duration::from_millis(20), line_rx.recv()).await {
            Ok(Some(output_line)) => {
                record_cursor_login_output(
                    &state,
                    &login_id,
                    output_line,
                    &mut transcript,
                    &mut observed_email,
                    &mut observed_auth_url,
                )
                .await;
            }
            Ok(None) => break,
            Err(_) if Instant::now() >= drain_deadline => break,
            Err(_) => {
                continue;
            }
        }
    }

    let mut final_status = "failed".to_string();
    let mut final_error = timeout_error;
    let mut final_account_id: Option<String> = None;

    if final_error.is_none() {
        match exit_result {
            Ok(status) if status.success() => {
                match parse_cursor_captured_tokens(&capture_path).await {
                    Ok((access_token, refresh_token, api_key)) => {
                        let auth_token = access_token.or(api_key);
                        if let Some(auth_token) = auth_token {
                            match provider_accounts::add_cursor_oauth_account(
                                &state.core.data_root,
                                label.clone(),
                                auth_token,
                                refresh_token,
                                observed_email,
                            )
                            .await
                            {
                                Ok(registry) => {
                                    final_account_id = registry.active_account_id.clone();
                                    match super::super::restarts::restart_cursor_providers_for_auth_change(
                                        &state,
                                        "cursor auth updated",
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
                        } else {
                            final_error = Some(
                                "Cursor login completed but no managed auth token was captured"
                                    .to_string(),
                            );
                        }
                    }
                    Err(err) => {
                        final_error = Some(logs::redact_sensitive(&err.to_string()));
                    }
                }
            }
            Ok(status) => {
                final_error = Some(format!("cursor-agent login exited with status {status}"));
            }
            Err(err) => {
                final_error = Some(format!("waiting for cursor-agent login failed: {err}"));
            }
        }
    }

    let _ = tokio::fs::remove_dir_all(&login_home).await;
    let mut map = state.providers.cursor_login_sessions.lock().await;
    if let Some(entry) = map.get_mut(&login_id) {
        entry.status = final_status;
        entry.account_id = final_account_id;
        entry.error = final_error;
        if entry.auth_url.is_none() {
            entry.auth_url = observed_auth_url;
        }
    }
}
