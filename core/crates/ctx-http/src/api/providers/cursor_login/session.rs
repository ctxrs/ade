use super::capture::parse_cursor_captured_tokens;
use super::output::{
    cursor_login_timeout, spawn_cursor_login_reader, CursorLoginOutputLine,
    CURSOR_LOGIN_POLL_INTERVAL,
};
use super::runtime::resolve_cursor_login_runtime;
use super::*;

mod completion;
mod progress;
#[path = "session/workspace.rs"]
mod workspace;

use completion::complete_cursor_login;
use progress::{record_cursor_login_output, set_cursor_login_error};
use workspace::prepare_cursor_login_workspace;

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

    let workspace = match prepare_cursor_login_workspace(&state.core.data_root, &login_id).await {
        Ok(workspace) => workspace,
        Err(err) => {
            let login_home = err.login_home().to_path_buf();
            set_cursor_login_error(&state, &login_id, err.into_status_error()).await;
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }
    };

    let hook_require = format!("--require {}", workspace.hook_path.to_string_lossy());
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
    cmd.current_dir(&workspace.workdir);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.env("NO_OPEN_BROWSER", "1");
    cmd.env(
        "CTX_CURSOR_CAPTURE_FILE",
        workspace.capture_path.to_string_lossy().to_string(),
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
            let _ = tokio::fs::remove_dir_all(&workspace.login_home).await;
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

    let completion = complete_cursor_login(
        &state,
        label,
        &workspace.capture_path,
        observed_email,
        timeout_error,
        exit_result,
    )
    .await;

    let _ = tokio::fs::remove_dir_all(&workspace.login_home).await;
    let mut map = state.providers.cursor_login_sessions.lock().await;
    if let Some(entry) = map.get_mut(&login_id) {
        entry.status = completion.status;
        entry.account_id = completion.account_id;
        entry.error = completion.error;
        if entry.auth_url.is_none() {
            entry.auth_url = observed_auth_url;
        }
    }
}
