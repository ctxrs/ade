use super::capture::parse_cursor_captured_tokens;
use super::runtime::resolve_cursor_login_runtime;
use super::*;

mod command;
mod completion;
mod output_loop;
mod progress;
#[path = "session/workspace.rs"]
mod workspace;

use command::spawn_cursor_login_child;
use completion::complete_cursor_login;
use output_loop::collect_cursor_login_output;
use progress::set_cursor_login_error;
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

    let mut child = match spawn_cursor_login_child(&cursor_runtime, &workspace) {
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

    let output = collect_cursor_login_output(&state, &login_id, &mut child).await;

    let completion = complete_cursor_login(
        &state,
        label,
        &workspace.capture_path,
        output.observed_email,
        output.timeout_error,
        output.exit_result,
    )
    .await;

    let _ = tokio::fs::remove_dir_all(&workspace.login_home).await;
    let mut map = state.providers.cursor_login_sessions.lock().await;
    if let Some(entry) = map.get_mut(&login_id) {
        entry.status = completion.status;
        entry.account_id = completion.account_id;
        entry.error = completion.error;
        if entry.auth_url.is_none() {
            entry.auth_url = output.observed_auth_url;
        }
    }
}
