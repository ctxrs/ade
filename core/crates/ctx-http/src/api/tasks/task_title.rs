use super::*;

#[path = "task_title/effects.rs"]
mod effects;

use effects::emit_task_title_update_effects;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct UpdateTaskTitleReq {
    title: String,
}

pub(in crate::api) async fn update_task_title(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTaskTitleReq>,
) -> Result<Json<Task>, (StatusCode, Json<ApiErrorResp>)> {
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

    let store = state.store_for_task(task_id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let updated = store.update_task_title(task_id, title).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    if !updated {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "task not found".to_string(),
            }),
        ));
    }

    let task = match store.get_task_with_activity(task_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })? {
        Some(task) => task,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "task not found".to_string(),
                }),
            ))
        }
    };

    emit_task_title_update_effects(&state, &store, &task).await;
    Ok(Json(task))
}
