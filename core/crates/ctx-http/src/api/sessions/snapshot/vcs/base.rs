use super::*;
use ctx_workspace_config as workspace_config;
pub(crate) use ctx_workspace_services::worktree_vcs::{
    is_no_vcs_repo_error, WorktreeDiffBaseResolution,
};

async fn resolve_worktree_ref_commits(
    state: &Arc<AppState>,
    worktree: &Worktree,
    target_branch: Option<&str>,
) -> (Option<String>, Option<String>) {
    let Some(target_branch) = target_branch else {
        return match crate::git_status::worktree_rev_parse_refs(state, worktree, &["HEAD"]).await {
            Ok(commits) => (commits.first().cloned(), None),
            Err(err) => {
                tracing::warn!(
                    worktree_id = %worktree.id.0,
                    "failed to resolve worktree head for diff metadata: {err:#}"
                );
                (None, None)
            }
        };
    };
    let refs = vec!["HEAD", target_branch];
    match crate::git_status::worktree_rev_parse_refs(state, worktree, &refs).await {
        Ok(commits) => {
            let head = commits.first().cloned();
            let target = commits.get(1).cloned();
            (head, target)
        }
        Err(err) => {
            tracing::warn!(
                worktree_id = %worktree.id.0,
                "failed to resolve worktree refs for diff metadata: {err:#}"
            );
            let head = match crate::git_status::worktree_rev_parse_head(state, worktree).await {
                Ok(head) => Some(head),
                Err(head_err) => {
                    tracing::warn!(
                        worktree_id = %worktree.id.0,
                        "failed to resolve worktree head after target branch lookup failed: {head_err:#}"
                    );
                    None
                }
            };
            (head, None)
        }
    }
}

pub(crate) async fn resolve_diff_base_with_meta(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    workspace: &Workspace,
    worktree: &Worktree,
    query: &SessionDiffQuery,
) -> WorktreeDiffBaseResolution {
    if let Some(base) = query.base_commit_sha.as_deref() {
        let trimmed = base.trim();
        if !trimmed.is_empty() {
            let (head_commit_sha, _) = resolve_worktree_ref_commits(state, worktree, None).await;
            return WorktreeDiffBaseResolution {
                base_commit_sha: trimmed.to_string(),
                head_commit_sha,
                target_branch: None,
                target_branch_commit_sha: None,
                target_source: None,
                kind: WorktreeVcsBaseResolutionKind::ExplicitBase,
                error: None,
                unavailable_reason: None,
                explicit_target: false,
            };
        }
    }

    let explicit_target = query
        .target_branch
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let mut target_branch = match query.target_branch.as_deref() {
        Some(target) if !target.trim().is_empty() => Some(target.trim().to_string()),
        _ => None,
    };
    let mut target_source = if target_branch.is_some() {
        Some(WorktreeVcsTargetSource::Explicit)
    } else {
        None
    };

    if target_branch.is_none() {
        match workspace_config::load_primary_branch(store).await {
            Ok(Some(branch)) => {
                target_branch = Some(branch);
                target_source = Some(WorktreeVcsTargetSource::PrimaryBranchConfig);
            }
            Ok(None) => {
                let (head_commit_sha, _) =
                    resolve_worktree_ref_commits(state, worktree, None).await;
                return WorktreeDiffBaseResolution {
                    base_commit_sha: worktree.base_commit_sha.clone(),
                    head_commit_sha,
                    target_branch: None,
                    target_branch_commit_sha: None,
                    target_source: None,
                    kind: WorktreeVcsBaseResolutionKind::WorktreeBase,
                    error: Some("workspace primary branch is not configured".to_string()),
                    unavailable_reason: Some(DiffUnavailableReason::NoTargetBranch),
                    explicit_target,
                };
            }
            Err(err) => tracing::warn!(
                workspace_id = %workspace.id.0,
                "failed to load workspace primary branch: {err:#}"
            ),
        }
    }

    let mut error: Option<String> = None;
    let mut unavailable_reason: Option<DiffUnavailableReason> = None;
    if let Some(target_branch) = target_branch.clone() {
        match crate::git_status::worktree_merge_base(state, worktree, &target_branch).await {
            Ok(base) => {
                let (head_commit_sha, target_branch_commit_sha) =
                    resolve_worktree_ref_commits(state, worktree, Some(&target_branch)).await;
                return WorktreeDiffBaseResolution {
                    base_commit_sha: base,
                    head_commit_sha,
                    target_branch: Some(target_branch),
                    target_branch_commit_sha,
                    target_source,
                    kind: WorktreeVcsBaseResolutionKind::MergeBase,
                    error: None,
                    unavailable_reason: None,
                    explicit_target,
                };
            }
            Err(err) => {
                let redacted = logs::redact_sensitive(&err.to_string());
                error = Some(redacted);
                unavailable_reason = Some(if is_no_vcs_repo_error(&err) {
                    DiffUnavailableReason::NoRepo
                } else {
                    DiffUnavailableReason::NoTargetBranch
                });
                tracing::warn!(
                    worktree_id = %worktree.id.0,
                    "merge-base failed for target {target_branch}: {err:#}"
                );
            }
        }
    }

    let (head_commit_sha, target_branch_commit_sha) =
        resolve_worktree_ref_commits(state, worktree, target_branch.as_deref()).await;
    WorktreeDiffBaseResolution {
        base_commit_sha: worktree.base_commit_sha.clone(),
        head_commit_sha,
        target_branch,
        target_branch_commit_sha,
        target_source,
        kind: WorktreeVcsBaseResolutionKind::WorktreeBase,
        error,
        unavailable_reason,
        explicit_target,
    }
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
