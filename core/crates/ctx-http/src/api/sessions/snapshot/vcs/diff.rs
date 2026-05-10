use super::context::load_session_vcs_context;
use super::*;
use crate::api::sessions::diff_exec::diff_worktree_for_session;
use ctx_workspace_services::worktree_vcs::{
    worktree_vcs_session_diff_available, worktree_vcs_session_diff_unavailable,
    WorktreeVcsSessionDiffOutcome,
};

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
        return Ok(Json(session_diff_summary_no_repo_response(
            ctx.worktree.base_commit_sha.clone(),
        )));
    }
    let resolution =
        resolve_session_diff_base(&state, &store, &ctx.workspace, &ctx.worktree, &q).await?;
    if let Some(unavailable_reason) = resolution.unavailable_reason.clone() {
        state
            .emit_compat_payload_reject_counter("sessions.diff_summary", "no_target_branch", None)
            .await;
        return Ok(Json(
            session_diff_summary_unavailable_response(
                &state,
                &ctx.worktree,
                resolution.base_commit_sha,
                unavailable_reason,
            )
            .await,
        ));
    }
    let base_commit_sha = resolution.base_commit_sha;
    Ok(Json(
        session_diff_summary_available_response(&state, &ctx.worktree, base_commit_sha).await?,
    ))
}

fn session_diff_response(outcome: WorktreeVcsSessionDiffOutcome) -> SessionDiffResponse {
    SessionDiffResponse {
        diff: outcome.diff,
        available: outcome.available,
        unavailable_reason: outcome.unavailable_reason,
    }
}
