use super::*;
use crate::api::sessions::diff_exec::diff_worktree_summary_for_session;
use crate::daemon::git_status::HttpWorktreeVcsSource;
use ctx_workspace_services::worktree_vcs::{
    is_no_vcs_repo_error, worktree_vcs_diff_summary_mismatch,
    worktree_vcs_session_diff_summary_available, worktree_vcs_session_diff_summary_no_repo,
    worktree_vcs_session_diff_summary_unavailable, WorktreeVcsCommitLookupSource,
    WorktreeVcsSessionDiffSummaryOutcome,
};

pub(super) fn session_diff_summary_no_repo_response(
    base_commit_sha: String,
) -> SessionDiffSummaryResponse {
    session_diff_summary_response(worktree_vcs_session_diff_summary_no_repo(base_commit_sha))
}

pub(super) async fn session_diff_summary_unavailable_response(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: String,
    unavailable_reason: DiffUnavailableReason,
) -> SessionDiffSummaryResponse {
    let head_commit_sha = resolve_head_commit_sha_or_base(state, worktree, &base_commit_sha).await;
    session_diff_summary_response(worktree_vcs_session_diff_summary_unavailable(
        base_commit_sha,
        head_commit_sha,
        unavailable_reason,
    ))
}

pub(super) async fn session_diff_summary_available_response(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: String,
) -> Result<SessionDiffSummaryResponse, (StatusCode, Json<ApiErrorResp>)> {
    let summary_counts =
        match diff_worktree_summary_for_session(state, worktree, &base_commit_sha).await {
            Ok(counts) => Ok(counts),
            Err(err) if is_no_vcs_repo_error(&err) => Err(DiffUnavailableReason::NoRepo),
            Err(err) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&err.to_string()),
                    }),
                ));
            }
        };
    let head_commit_sha = resolve_head_commit_sha_or_base(state, worktree, &base_commit_sha).await;
    let outcome = match summary_counts {
        Ok(counts) => {
            if let Some(snapshot) = state.get_worktree_vcs_snapshot(worktree.id).await {
                if let Some(mismatch) =
                    worktree_vcs_diff_summary_mismatch(&snapshot, &base_commit_sha, counts)
                {
                    tracing::warn!(
                        worktree_id = %worktree.id.0,
                        snapshot_rev = snapshot.rev,
                        base_commit_sha = %base_commit_sha,
                        snapshot_file_count = ?mismatch.snapshot_file_count,
                        snapshot_additions = ?mismatch.snapshot_line_additions,
                        snapshot_deletions = ?mismatch.snapshot_line_deletions,
                        summary_file_count = mismatch.actual_file_count,
                        summary_additions = mismatch.actual_line_additions,
                        summary_deletions = mismatch.actual_line_deletions,
                        "worktree vcs snapshot summary mismatch"
                    );
                }
            }
            worktree_vcs_session_diff_summary_available(base_commit_sha, head_commit_sha, counts)
        }
        Err(unavailable_reason) => worktree_vcs_session_diff_summary_unavailable(
            base_commit_sha,
            head_commit_sha,
            unavailable_reason,
        ),
    };
    Ok(session_diff_summary_response(outcome))
}

async fn resolve_head_commit_sha_or_base(
    state: &Arc<AppState>,
    worktree: &Worktree,
    base_commit_sha: &str,
) -> String {
    let source = HttpWorktreeVcsSource::new(state, worktree);
    source
        .resolve_commit("HEAD")
        .await
        .unwrap_or_else(|_| base_commit_sha.to_string())
}

fn session_diff_summary_response(
    outcome: WorktreeVcsSessionDiffSummaryOutcome,
) -> SessionDiffSummaryResponse {
    SessionDiffSummaryResponse {
        base_commit_sha: outcome.base_commit_sha,
        head_commit_sha: outcome.head_commit_sha,
        file_count: outcome.counts.file_count,
        line_additions: outcome.counts.line_additions,
        line_deletions: outcome.counts.line_deletions,
        available: outcome.available,
        unavailable_reason: outcome.unavailable_reason,
    }
}
