use super::*;

pub(super) struct SessionVcsContext {
    pub(super) session: Session,
    pub(super) worktree: Worktree,
}

fn internal_error(err: impl ToString) -> (StatusCode, Json<ApiErrorResp>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiErrorResp {
            error: logs::redact_sensitive(&err.to_string()),
        }),
    )
}

pub(super) async fn load_session_vcs_context(
    state: &SessionsHandle,
    session_id: SessionId,
) -> Result<SessionVcsContext, (StatusCode, Json<ApiErrorResp>)> {
    let (session, worktree) = state
        .load_session_vcs_parts(session_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "workspace not found".to_string(),
                }),
            )
        })?;
    Ok(SessionVcsContext { session, worktree })
}
