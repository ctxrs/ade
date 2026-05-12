use std::sync::Arc;

use axum::http::StatusCode;
use ctx_core::models::{ExecutionEnvironment, Worktree};
use ctx_workspace_services::file_completions::merge_and_sort_git_paths;
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;
use ctx_worktree_data_plane::resolve_worktree_data_plane_with_host as resolve_worktree_data_plane;

use crate::api::shared::errors::status_code_for_internal_error;
use crate::daemon::execution_effective;
use crate::daemon::AppState;

#[path = "container_git.rs"]
mod container_git;

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
    let tracked = container_git::container_git_ls_files(
        state,
        worktree,
        settings.container.runtime.clone(),
        &workdir,
        &["ls-files", "-z"],
    )
    .await?;
    let untracked = container_git::container_git_ls_files(
        state,
        worktree,
        settings.container.runtime,
        &workdir,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )
    .await?;

    Ok(merge_and_sort_git_paths(tracked, untracked))
}
