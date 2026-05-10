use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use ctx_core::ids::{MergeQueueEntryId, WorkspaceId};
use ctx_core::models::MergeQueueEntry;

use super::request::MergeQueueListParams;
use crate::api::errors::ApiErrorResp;
use crate::api::shared::store_for_existing_workspace_status;
use crate::daemon::{merge_queue, AppState};

pub(in crate::api) async fn list_merge_queue_entries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MergeQueueListParams>,
) -> Result<Json<Vec<MergeQueueEntry>>, StatusCode> {
    let workspace_id = WorkspaceId(
        uuid::Uuid::parse_str(&params.workspace_id).map_err(|_| StatusCode::BAD_REQUEST)?,
    );
    let store = store_for_existing_workspace_status(&state, workspace_id).await?;
    let entries = store
        .list_merge_queue_entries(workspace_id, params.limit)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(entries))
}

pub(in crate::api) async fn cancel_merge_queue_entry(
    State(state): State<Arc<AppState>>,
    Path((workspace_id, id)): Path<(String, String)>,
) -> Result<Json<MergeQueueEntry>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = parse_workspace_id(&workspace_id)?;
    let entry_id = parse_entry_id(&id)?;
    let entry = merge_queue::cancel_merge_queue_entry(&state, workspace_id, entry_id)
        .await
        .map_err(|err| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: err.to_string(),
                }),
            )
        })?;
    Ok(Json(entry))
}

pub(in crate::api) async fn retry_merge_queue_entry(
    State(state): State<Arc<AppState>>,
    Path((workspace_id, id)): Path<(String, String)>,
) -> Result<Json<MergeQueueEntry>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = parse_workspace_id(&workspace_id)?;
    let entry_id = parse_entry_id(&id)?;
    let entry = merge_queue::retry_merge_queue_entry(&state, workspace_id, entry_id)
        .await
        .map_err(|err| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: err.to_string(),
                }),
            )
        })?;
    Ok(Json(entry))
}

fn parse_workspace_id(value: &str) -> Result<WorkspaceId, (StatusCode, Json<ApiErrorResp>)> {
    uuid::Uuid::parse_str(value)
        .map(WorkspaceId)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid workspace id"))
}

fn parse_entry_id(value: &str) -> Result<MergeQueueEntryId, (StatusCode, Json<ApiErrorResp>)> {
    uuid::Uuid::parse_str(value)
        .map(MergeQueueEntryId)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid entry id"))
}

fn api_error(status: StatusCode, error: &str) -> (StatusCode, Json<ApiErrorResp>) {
    (
        status,
        Json(ApiErrorResp {
            error: error.to_string(),
        }),
    )
}
