use super::context::load_session_vcs_context;
use super::*;
use crate::api::sessions::diff_exec::diff_worktree_for_session;
use crate::daemon::git_status::HttpWorktreeVcsSource;
use ctx_workspace_services::worktree_vcs::{
    worktree_vcs_diff_summary_mismatch, worktree_vcs_session_diff_available,
    worktree_vcs_session_diff_summary_available, worktree_vcs_session_diff_summary_no_repo,
    worktree_vcs_session_diff_summary_unavailable, worktree_vcs_session_diff_unavailable,
    WorktreeVcsCommitLookupSource, WorktreeVcsSessionDiffOutcome,
    WorktreeVcsSessionDiffSummaryOutcome,
};

pub(crate) async fn get_session_diff(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionDiffQuery>,
) -> Result<Json<SessionDiffResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);
    let (store, ctx) = load_session_vcs_context(&state, session_id).await?;
    if !crate::daemon::git_status::worktree_has_vcs_repo(&state, &ctx.worktree)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&err.to_string()),
                }),
            )
        })?
    {
        return Ok(Json(session_diff_response(
            worktree_vcs_session_diff_unavailable(DiffUnavailableReason::NoRepo),
        )));
    }
    let resolution =
        resolve_session_diff_base(&state, &store, &ctx.workspace, &ctx.worktree, &q).await?;
    if let Some(unavailable_reason) = resolution.unavailable_reason.clone() {
        state
            .emit_compat_payload_reject_counter("sessions.diff", "no_target_branch", None)
            .await;
        return Ok(Json(session_diff_response(
            worktree_vcs_session_diff_unavailable(unavailable_reason),
        )));
    }
    let base_commit_sha = resolution.base_commit_sha;
    let diff = match diff_worktree_for_session(&state, &ctx.worktree, &base_commit_sha).await {
        Ok(diff) => diff,
        Err(err) if is_no_vcs_repo_error(&err) => {
            return Ok(Json(session_diff_response(
                worktree_vcs_session_diff_unavailable(DiffUnavailableReason::NoRepo),
            )));
        }
        Err(err) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&err.to_string()),
                }),
            ));
        }
    };
    Ok(Json(session_diff_response(
        worktree_vcs_session_diff_available(diff),
    )))
}

pub(crate) async fn get_session_diff_summary(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionDiffQuery>,
) -> Result<Json<SessionDiffSummaryResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);
    let (store, ctx) = load_session_vcs_context(&state, session_id).await?;
    if !crate::daemon::git_status::worktree_has_vcs_repo(&state, &ctx.worktree)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&err.to_string()),
                }),
            )
        })?
    {
        return Ok(Json(session_diff_summary_response(
            worktree_vcs_session_diff_summary_no_repo(ctx.worktree.base_commit_sha.clone()),
        )));
    }
    let resolution =
        resolve_session_diff_base(&state, &store, &ctx.workspace, &ctx.worktree, &q).await?;
    if let Some(unavailable_reason) = resolution.unavailable_reason.clone() {
        state
            .emit_compat_payload_reject_counter("sessions.diff_summary", "no_target_branch", None)
            .await;
        let source = HttpWorktreeVcsSource::new(&state, &ctx.worktree);
        let head_commit_sha = source
            .resolve_commit("HEAD")
            .await
            .unwrap_or_else(|_| resolution.base_commit_sha.clone());
        return Ok(Json(session_diff_summary_response(
            worktree_vcs_session_diff_summary_unavailable(
                resolution.base_commit_sha,
                head_commit_sha,
                unavailable_reason,
            ),
        )));
    }
    let base_commit_sha = resolution.base_commit_sha;
    let summary_counts =
        match diff_worktree_summary_for_session(&state, &ctx.worktree, &base_commit_sha).await {
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
    let source = HttpWorktreeVcsSource::new(&state, &ctx.worktree);
    let head_commit_sha = source
        .resolve_commit("HEAD")
        .await
        .unwrap_or_else(|_| base_commit_sha.clone());
    let outcome = match summary_counts {
        Ok(counts) => {
            if let Some(snapshot) = state.get_worktree_vcs_snapshot(ctx.worktree.id).await {
                if let Some(mismatch) =
                    worktree_vcs_diff_summary_mismatch(&snapshot, &base_commit_sha, counts)
                {
                    tracing::warn!(
                        worktree_id = %ctx.worktree.id.0,
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
    Ok(Json(session_diff_summary_response(outcome)))
}

fn session_diff_response(outcome: WorktreeVcsSessionDiffOutcome) -> SessionDiffResponse {
    SessionDiffResponse {
        diff: outcome.diff,
        available: outcome.available,
        unavailable_reason: outcome.unavailable_reason,
    }
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
