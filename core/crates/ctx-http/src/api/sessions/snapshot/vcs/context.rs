use super::*;

pub(super) struct SessionVcsContext {
    pub(super) session: Session,
    pub(super) worktree: Worktree,
    pub(super) workspace: Workspace,
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
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<(ctx_store::Store, SessionVcsContext), (StatusCode, Json<ApiErrorResp>)> {
    let store = store_for_existing_session_api_error(state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "session not found".to_string(),
                }),
            )
        })?;
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "worktree not found".to_string(),
                }),
            )
        })?;
    let workspace = state
        .global_store()
        .get_workspace(session.workspace_id)
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
    Ok((
        store,
        SessionVcsContext {
            session,
            worktree,
            workspace,
        },
    ))
}
