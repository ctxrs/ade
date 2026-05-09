use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use ctx_core::models::{ExecutionEnvironment, Worktree};
use ctx_settings_model::ContainerRuntimeKind;
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;

use crate::api::shared::errors::status_code_for_internal_error;
use crate::daemon::execution_effective;
use crate::daemon::AppState;

use super::listing::merge_and_sort_git_paths;

pub(super) async fn list_container_worktree_files(
    state: &Arc<AppState>,
    worktree: &Worktree,
    execution_environment: ExecutionEnvironment,
) -> Result<Vec<String>, StatusCode> {
    let workspace_id = worktree.workspace_id;
    let workspace = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let settings = execution_effective::effective_execution_settings_for_environment(
        state,
        workspace_id,
        execution_environment,
    )
    .await
    .map_err(|err| status_code_for_internal_error(&err))?;
    let data_plane = resolve_worktree_data_plane(state.as_ref(), worktree)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let settings = apply_data_plane_to_execution_settings(&settings, &data_plane)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .execution
        .harness
        .ensure_workspace_container_for_worktree(
            &workspace,
            worktree,
            &settings,
            &state.core.daemon_url,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let workdir = data_plane.live_worktree_root.to_string_lossy().to_string();
    let tracked = container_git_ls_files(
        state,
        worktree,
        settings.container.runtime.clone(),
        &workdir,
        &["ls-files", "-z"],
    )
    .await?;
    let untracked = container_git_ls_files(
        state,
        worktree,
        settings.container.runtime,
        &workdir,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )
    .await?;

    Ok(merge_and_sort_git_paths(tracked, untracked))
}

async fn container_git_ls_files(
    state: &Arc<AppState>,
    worktree: &Worktree,
    runtime: ContainerRuntimeKind,
    workdir: &str,
    git_args: &[&str],
) -> Result<Vec<String>, StatusCode> {
    const SANDBOX_GIT_LS_FILES_TIMEOUT: Duration = Duration::from_secs(30);
    let out = match runtime {
        ContainerRuntimeKind::NativeContainer => {
            let mut cmd = ctx_harness_runtime::sandbox_container_command(&state.core.data_root)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            cmd.arg("exec")
                .arg("--workdir")
                .arg(workdir)
                .arg(ctx_workspace_container::workspace_container_name(
                    worktree.workspace_id,
                ))
                .arg("git");
            for arg in git_args {
                cmd.arg(arg);
            }
            ctx_sandbox_container_runtime::command_output_with_timeout(
                cmd,
                SANDBOX_GIT_LS_FILES_TIMEOUT,
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        }
        ContainerRuntimeKind::SharedVmContainer => {
            let args = git_args
                .iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>();
            let guest_cwd = PathBuf::from(workdir);
            tokio::time::timeout(
                SANDBOX_GIT_LS_FILES_TIMEOUT,
                ctx_avf_linux_runtime::run_guest_exec_capture(
                    &state.core.data_root,
                    worktree.workspace_id,
                    worktree.id,
                    &guest_cwd,
                    "git",
                    &args,
                    &HashMap::new(),
                    None,
                    false,
                ),
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        }
    };
    if !out.status.success() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let mut files = Vec::new();
    for part in out.stdout.split(|b| *b == 0u8) {
        if part.is_empty() {
            continue;
        }
        let s = String::from_utf8_lossy(part).to_string();
        if !s.trim().is_empty() {
            files.push(s);
        }
    }
    Ok(files)
}
