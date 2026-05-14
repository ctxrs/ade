use super::*;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct CreateWorkspaceAttachmentReq {
    pub(super) kind: WorkspaceAttachmentKind,
    pub(super) name: String,
    pub(super) source: String,
    #[serde(default)]
    pub(super) revision: Option<String>,
    #[serde(default)]
    pub(super) subpath: Option<String>,
    #[serde(default)]
    pub(super) mount_relpath: Option<String>,
    #[serde(default)]
    pub(super) mode: Option<AttachmentMode>,
    #[serde(default)]
    pub(super) update_policy: Option<AttachmentUpdatePolicy>,
}

pub(in crate::api) async fn create_workspace_attachment(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
    Json(req): Json<CreateWorkspaceAttachmentReq>,
) -> Result<Json<Vec<WorkspaceAttachment>>, (StatusCode, Json<ApiErrorResp>)> {
    if req.name.trim().is_empty() || req.source.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "name and source are required".to_string(),
            }),
        ));
    }
    let ctx = require_workspace_ctx(&workspaces, &id).await?;
    create_and_sync_workspace_attachment(&workspaces, &ctx, req)
        .await
        .map(Json)
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct DeleteWorkspaceAttachmentReq {
    pub(super) kind: WorkspaceAttachmentKind,
    pub(super) name: String,
}

pub(in crate::api) async fn delete_workspace_attachment(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
    Json(req): Json<DeleteWorkspaceAttachmentReq>,
) -> Result<Json<Vec<WorkspaceAttachment>>, (StatusCode, Json<ApiErrorResp>)> {
    if req.name.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "name is required".to_string(),
            }),
        ));
    }
    let ctx = require_workspace_ctx(&workspaces, &id).await?;
    delete_and_sync_workspace_attachment(&workspaces, &ctx, req)
        .await
        .map(Json)
}
