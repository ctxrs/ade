use super::*;
use crate::daemon::git_status::HttpWorktreeVcsSource;
pub(crate) use ctx_workspace_services::worktree_vcs::{
    is_no_vcs_repo_error, resolve_worktree_diff_base_from_source, WorktreeDiffBaseResolution,
    WorktreeVcsDiffBaseQuery,
};

pub(crate) async fn resolve_diff_base_with_meta(
    state: &Arc<AppState>,
    _store: &ctx_store::Store,
    _workspace: &Workspace,
    worktree: &Worktree,
    query: &SessionDiffQuery,
) -> WorktreeDiffBaseResolution {
    let source = HttpWorktreeVcsSource::new(state, worktree);
    resolve_worktree_diff_base_from_source(
        &source,
        worktree,
        WorktreeVcsDiffBaseQuery {
            base_commit_sha: query.base_commit_sha.clone(),
            target_branch: query.target_branch.clone(),
        },
    )
    .await
}

pub(crate) async fn resolve_session_diff_base(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    workspace: &Workspace,
    worktree: &Worktree,
    query: &SessionDiffQuery,
) -> Result<WorktreeDiffBaseResolution, (StatusCode, Json<ApiErrorResp>)> {
    let resolution = resolve_diff_base_with_meta(state, store, workspace, worktree, query).await;
    if resolution.explicit_target {
        if let Some(error) = resolution.error.clone() {
            return Err((StatusCode::BAD_REQUEST, Json(ApiErrorResp { error })));
        }
    }
    Ok(resolution)
}
