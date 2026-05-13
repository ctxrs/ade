use super::*;

pub(super) type WorkspaceApiResult<T> = Result<T, (StatusCode, Json<ApiErrorResp>)>;

#[derive(Clone)]
pub(super) struct WorkspaceRequestContext {
    pub(super) workspace_id: WorkspaceId,
    pub(super) workspace: Workspace,
    pub(super) store: ctx_store::Store,
}

pub(super) fn parse_workspace_id(id: &str) -> WorkspaceApiResult<WorkspaceId> {
    Ok(WorkspaceId(uuid::Uuid::parse_str(id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?))
}

pub(super) async fn require_workspace(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> WorkspaceApiResult<Workspace> {
    state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))
}

pub(super) async fn require_workspace_store(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> WorkspaceApiResult<ctx_store::Store> {
    state
        .existing_workspace_store(workspace_id)
        .await
        .map_err(workspace_store_api_error)
}

fn workspace_store_api_error(
    error: crate::daemon::WorkspaceStoreAccessError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        crate::daemon::WorkspaceStoreAccessError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ),
        crate::daemon::WorkspaceStoreAccessError::Unavailable(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&error.to_string()),
            }),
        ),
    }
}

pub(super) async fn require_workspace_ctx(
    state: &Arc<AppState>,
    id: &str,
) -> WorkspaceApiResult<WorkspaceRequestContext> {
    let workspace_id = parse_workspace_id(id)?;
    let workspace = require_workspace(state, workspace_id).await?;
    let store = require_workspace_store(state, workspace_id).await?;
    Ok(WorkspaceRequestContext {
        workspace_id,
        workspace,
        store,
    })
}
