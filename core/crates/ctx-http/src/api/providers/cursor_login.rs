use super::login::{extract_auth_url, reject_mobile_auth};
use super::*;
use crate::api::MobileAuthContext;
use axum::Extension;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

mod capture;
mod runtime;
#[cfg(test)]
mod tests;

use capture::{
    cursor_login_home, ensure_private_dir, initialize_cursor_capture_file,
    parse_cursor_captured_tokens, write_cursor_capture_hook,
};
use runtime::resolve_cursor_login_runtime;
#[cfg(test)]
use runtime::resolve_cursor_login_runtime_from_config;

#[derive(Debug, Deserialize)]
pub(crate) struct CursorLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CursorLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

#[derive(Debug)]
struct CursorLoginOutputLine {
    line: String,
    is_stderr: bool,
}

const CURSOR_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const CURSOR_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);

fn cursor_login_timeout() -> Duration {
    let seconds = std::env::var("CTX_CURSOR_LOGIN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(CURSOR_LOGIN_TIMEOUT_DEFAULT.as_secs());
    Duration::from_secs(seconds)
}

fn first_email_from_text(value: &str) -> Option<String> {
    value
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|ch: char| {
                    !ch.is_ascii_alphanumeric()
                        && ch != '@'
                        && ch != '.'
                        && ch != '_'
                        && ch != '-'
                        && ch != '+'
                })
                .to_string()
        })
        .find(|token| {
            let Some((local, domain)) = token.split_once('@') else {
                return false;
            };
            !local.is_empty() && domain.contains('.')
        })
}

fn spawn_cursor_login_reader<R>(
    reader: R,
    is_stderr: bool,
    tx: mpsc::UnboundedSender<CursorLoginOutputLine>,
) where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if tx.send(CursorLoginOutputLine { line, is_stderr }).is_err() {
                        return;
                    }
                }
                Ok(None) | Err(_) => return,
            }
        }
    });
}

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

async fn monitor_cursor_login(state: Arc<AppState>, login_id: String, label: Option<String>) {
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
                    transcript.push_str(&output_line.line);
                    transcript.push('\n');
                    if !output_line.is_stderr && observed_email.is_none() {
                        observed_email = first_email_from_text(&output_line.line);
                    }
                    if let Some(candidate) =
                        extract_auth_url(&output_line.line).or_else(|| extract_auth_url(&transcript))
                    {
                        let needs_update = observed_auth_url
                            .as_ref()
                            .is_none_or(|current| candidate.len() > current.len());
                        if needs_update {
                            observed_auth_url = Some(candidate.clone());
                            update_cursor_auth_url(&state, &login_id, candidate).await;
                        }
                    }
                }
            }
            wait = child.wait() => {
                break wait;
            }
            _ = tokio::time::sleep(CURSOR_LOGIN_POLL_INTERVAL) => {}
        }
    };

    while let Ok(output_line) = line_rx.try_recv() {
        transcript.push_str(&output_line.line);
        transcript.push('\n');
        if !output_line.is_stderr && observed_email.is_none() {
            observed_email = first_email_from_text(&output_line.line);
        }
        if let Some(candidate) =
            extract_auth_url(&output_line.line).or_else(|| extract_auth_url(&transcript))
        {
            let needs_update = observed_auth_url
                .as_ref()
                .is_none_or(|current| candidate.len() > current.len());
            if needs_update {
                observed_auth_url = Some(candidate.clone());
                update_cursor_auth_url(&state, &login_id, candidate).await;
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
                                    match super::restarts::restart_cursor_providers_for_auth_change(
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

pub(crate) async fn start_cursor_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<CursorLoginStartReq>,
) -> Result<Json<CursorLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let _ = resolve_cursor_login_runtime(&state).await.map_err(|e| {
        let msg = e.to_string();
        let status = if msg.contains("runtime_command_") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        (status, Json(ApiErrorResp { error: msg }))
    })?;

    let login_id = uuid::Uuid::new_v4().to_string();
    {
        let mut map = state.providers.cursor_login_sessions.lock().await;
        map.insert(
            login_id.clone(),
            provider_accounts::CursorLoginStatus {
                login_id: login_id.clone(),
                auth_url: None,
                status: "pending".to_string(),
                account_id: None,
                error: None,
            },
        );
    }

    let state_clone = Arc::clone(&state);
    let login_id_for_task = login_id.clone();
    tokio::spawn(async move {
        monitor_cursor_login(state_clone, login_id_for_task, req.label).await;
    });

    Ok(Json(CursorLoginStartResp {
        login_id,
        auth_url: None,
    }))
}

pub(crate) async fn get_cursor_login(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::CursorLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    reject_mobile_auth(mobile_auth)?;
    let map = state.providers.cursor_login_sessions.lock().await;
    let status = map.get(&id).cloned().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "login not found".to_string(),
            }),
        )
    })?;
    Ok(Json(status))
}
