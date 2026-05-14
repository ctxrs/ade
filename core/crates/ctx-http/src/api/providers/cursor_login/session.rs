use super::capture::parse_cursor_captured_tokens;
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
use workspace::prepare_cursor_login_workspace;

pub(super) async fn monitor_cursor_login(
    providers: ProvidersHandle,
    login_id: String,
    label: Option<String>,
) {
    let cursor_runtime = match providers.resolve_cursor_login_runtime().await {
        Ok(runtime) => runtime,
        Err(err) => {
            providers
                .set_cursor_login_error(&login_id, logs::redact_sensitive(&err.to_string()))
                .await;
            return;
        }
    };

    let workspace = match prepare_cursor_login_workspace(providers.data_root(), &login_id).await {
        Ok(workspace) => workspace,
        Err(err) => {
            let login_home = err.login_home().to_path_buf();
            providers
                .set_cursor_login_error(&login_id, err.into_status_error())
                .await;
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }
    };

    let mut child = match spawn_cursor_login_child(&cursor_runtime, &workspace) {
        Ok(child) => child,
        Err(err) => {
            providers
                .set_cursor_login_error(
                    &login_id,
                    format!("failed to launch cursor-agent login: {err}"),
                )
                .await;
            let _ = tokio::fs::remove_dir_all(&workspace.login_home).await;
            return;
        }
    };

    let output = collect_cursor_login_output(&providers, &login_id, &mut child).await;

    let completion = complete_cursor_login(
        &providers,
        label,
        &workspace.capture_path,
        output.observed_email,
        output.timeout_error,
        output.exit_result,
    )
    .await;

    let _ = tokio::fs::remove_dir_all(&workspace.login_home).await;
    providers
        .finish_cursor_login_session(
            &login_id,
            completion.status,
            completion.account_id,
            completion.error,
            output.observed_auth_url,
        )
        .await;
}
