use super::*;
use ctx_workspace_services::worktree_vcs::{WorktreeDiffBaseResolution, WorktreeVcsDiffBaseQuery};

pub(crate) async fn resolve_diff_base_with_meta(
    state: &SessionsHandle,
    worktree: &Worktree,
    query: &SessionDiffQuery,
) -> WorktreeDiffBaseResolution {
    state
        .resolve_worktree_diff_base(
            worktree,
            WorktreeVcsDiffBaseQuery {
                base_commit_sha: query.base_commit_sha.clone(),
                target_branch: query.target_branch.clone(),
            },
        )
        .await
}

pub(crate) async fn resolve_session_diff_base(
    state: &SessionsHandle,
    worktree: &Worktree,
    query: &SessionDiffQuery,
) -> Result<WorktreeDiffBaseResolution, (StatusCode, Json<ApiErrorResp>)> {
    let resolution = resolve_diff_base_with_meta(state, worktree, query).await;
    if resolution.explicit_target {
        if let Some(error) = resolution.error.clone() {
            return Err((StatusCode::BAD_REQUEST, Json(ApiErrorResp { error })));
        }
    }
    Ok(resolution)
}
