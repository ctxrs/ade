use super::*;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct UpdateTaskTitleReq {
    title: String,
}

pub(in crate::api) async fn update_task_title(
    State(sessions): State<SessionsHandle>,
    State(providers): State<ProvidersHandle>,
    State(workspaces): State<WorkspacesHandle>,
    State(transport): State<TransportHandle>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTaskTitleReq>,
) -> Result<Json<Task>, (StatusCode, Json<ApiErrorResp>)> {
    let handles = TaskApiHandles::new(sessions, providers, workspaces, transport);
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid task id".to_string(),
            }),
        )
    })?);
    let title = req.title.trim().to_string();
    if title.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "title is required".to_string(),
            }),
        ));
    }
    if title.len() > 120 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "title is too long".to_string(),
            }),
        ));
    }

    let update = handles
        .sessions
        .update_task_title(task_id, title)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let update = match update {
        Some(update) => update,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "task not found".to_string(),
                }),
            ))
        }
    };

    let _ = handles
        .workspaces
        .emit_workspace_task_delta(update.task.clone(), TaskDeltaKind::Updated)
        .await;
    if let Err(e) = handles.workspaces.emit_workspace_task_upsert(task_id).await {
        tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }
    if let Err(e) = handles
        .transport
        .close_web_sessions_for_task(&update.session_ids, &update.worktree_ids)
        .await
    {
        tracing::warn!(task_id = %task_id.0, "failed to close web sessions for title update: {e:?}");
    }
    Ok(Json(update.task))
}
