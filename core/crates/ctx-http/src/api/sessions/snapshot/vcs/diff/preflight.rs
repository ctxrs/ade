use super::super::context::{load_session_vcs_context, SessionVcsContext};
use super::*;

pub(super) enum PreparedSessionDiffRequest {
    Available {
        ctx: SessionVcsContext,
        base_commit_sha: String,
    },
    NoRepo {
        ctx: SessionVcsContext,
    },
    Unavailable {
        ctx: SessionVcsContext,
        base_commit_sha: String,
        reason: DiffUnavailableReason,
    },
}

pub(super) async fn prepare_session_diff_request(
    state: &Arc<AppState>,
    id: &str,
    query: &SessionDiffQuery,
    compat_route: &'static str,
) -> Result<PreparedSessionDiffRequest, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);
    let (store, ctx) = load_session_vcs_context(state, session_id).await?;
    if !crate::daemon::git_status::worktree_has_vcs_repo(state, &ctx.worktree)
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
        return Ok(PreparedSessionDiffRequest::NoRepo { ctx });
    }
    let resolution =
        resolve_session_diff_base(state, &store, &ctx.workspace, &ctx.worktree, query).await?;
    if let Some(reason) = resolution.unavailable_reason.clone() {
        state
            .emit_compat_payload_reject_counter(compat_route, "no_target_branch", None)
            .await;
        return Ok(PreparedSessionDiffRequest::Unavailable {
            ctx,
            base_commit_sha: resolution.base_commit_sha,
            reason,
        });
    }
    Ok(PreparedSessionDiffRequest::Available {
        ctx,
        base_commit_sha: resolution.base_commit_sha,
    })
}
