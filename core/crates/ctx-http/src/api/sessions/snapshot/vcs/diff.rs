use super::*;
use crate::daemon::workspaces::diff_worktree_for_session;
use ctx_workspace_services::worktree_vcs::{
    is_no_vcs_repo_error, worktree_vcs_session_diff_available,
    worktree_vcs_session_diff_unavailable, WorktreeVcsSessionDiffOutcome,
};
use preflight::{prepare_session_diff_request, PreparedSessionDiffRequest};

#[path = "diff/preflight.rs"]
mod preflight;
#[path = "diff/summary.rs"]
mod summary;

use self::summary::{
    session_diff_summary_available_response, session_diff_summary_no_repo_response,
    session_diff_summary_unavailable_response,
};

pub(crate) async fn get_session_diff(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionDiffQuery>,
) -> Result<Json<SessionDiffResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let (ctx, base_commit_sha) =
        match prepare_session_diff_request(&state, &id, &q, "sessions.diff").await? {
            PreparedSessionDiffRequest::Available {
                ctx,
                base_commit_sha,
            } => (ctx, base_commit_sha),
            PreparedSessionDiffRequest::NoRepo { .. } => {
                return Ok(Json(session_diff_response(
                    worktree_vcs_session_diff_unavailable(DiffUnavailableReason::NoRepo),
                )));
            }
            PreparedSessionDiffRequest::Unavailable { reason, .. } => {
                return Ok(Json(session_diff_response(
                    worktree_vcs_session_diff_unavailable(reason),
                )));
            }
        };
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
    match prepare_session_diff_request(&state, &id, &q, "sessions.diff_summary").await? {
        PreparedSessionDiffRequest::Available {
            ctx,
            base_commit_sha,
        } => Ok(Json(
            session_diff_summary_available_response(&state, &ctx.worktree, base_commit_sha).await?,
        )),
        PreparedSessionDiffRequest::NoRepo { ctx } => Ok(Json(
            session_diff_summary_no_repo_response(ctx.worktree.base_commit_sha.clone()),
        )),
        PreparedSessionDiffRequest::Unavailable {
            ctx,
            base_commit_sha,
            reason,
        } => Ok(Json(
            session_diff_summary_unavailable_response(
                &state,
                &ctx.worktree,
                base_commit_sha,
                reason,
            )
            .await,
        )),
    }
}

fn session_diff_response(outcome: WorktreeVcsSessionDiffOutcome) -> SessionDiffResponse {
    SessionDiffResponse {
        diff: outcome.diff,
        available: outcome.available,
        unavailable_reason: outcome.unavailable_reason,
    }
}
