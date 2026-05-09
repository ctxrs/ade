use super::*;

pub(super) struct CreateTaskRequestParts {
    pub(super) task_id: Option<TaskId>,
    pub(super) requested_title: String,
    pub(super) requested_description: Option<String>,
    pub(super) requested_default_session: Option<CreateTaskDefaultSessionReq>,
}

impl CreateTaskRequestParts {
    pub(super) fn from_request(req: CreateTaskReq) -> Result<Self, CreateTaskApiError> {
        let task_id = match req.id.as_deref().map(str::trim) {
            Some("") | None => None,
            Some(raw) => Some(TaskId(uuid::Uuid::parse_str(raw).map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: "invalid task id".to_string(),
                    }),
                )
            })?)),
        };
        Ok(Self {
            task_id,
            requested_title: req.title,
            requested_description: req.description,
            requested_default_session: req.default_session,
        })
    }

    pub(super) fn should_preflight_default_session(&self, existing_task: &Option<Task>) -> bool {
        existing_task.is_none() && self.requested_default_session.is_none()
    }
}

pub(super) struct PersistedTaskForCreate {
    pub(super) task: Task,
    pub(super) created_in_this_request: bool,
}

pub(super) async fn load_existing_task_for_request(
    state: &Arc<AppState>,
    store: &Store,
    ws_id: WorkspaceId,
    request: &CreateTaskRequestParts,
) -> Result<Option<Task>, CreateTaskApiError> {
    let Some(task_id) = request.task_id else {
        return Ok(None);
    };
    let existing_ws = state
        .global_store()
        .get_workspace_id_for_task(task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let Some(existing_ws) = existing_ws else {
        return Ok(None);
    };
    if existing_ws != ws_id {
        return Err(task_id_conflict());
    }
    let existing = store.get_task(task_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let Some(existing) = existing else {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: "task index exists but task missing".to_string(),
            }),
        ));
    };
    validate_requested_task_identity(&existing, ws_id, request)?;
    Ok(Some(existing))
}

pub(super) async fn persist_task_for_request(
    store: &Store,
    ws_id: WorkspaceId,
    existing_task: Option<Task>,
    request: &CreateTaskRequestParts,
) -> Result<PersistedTaskForCreate, CreateTaskApiError> {
    let (task, created_in_this_request) = match existing_task {
        Some(existing) => (existing, false),
        None => match request.task_id {
            Some(task_id) => {
                let result = store
                    .create_task_with_id_result(
                        ws_id,
                        task_id,
                        request.requested_title.clone(),
                        request.requested_description.clone(),
                    )
                    .await
                    .map_err(internal_store_error)?;
                (result.task, result.created)
            }
            None => (
                store
                    .create_task(
                        ws_id,
                        request.requested_title.clone(),
                        request.requested_description.clone(),
                    )
                    .await
                    .map_err(internal_store_error)?,
                true,
            ),
        },
    };
    validate_requested_task_identity(&task, ws_id, request)?;
    Ok(PersistedTaskForCreate {
        task,
        created_in_this_request,
    })
}

pub(super) async fn upsert_workspace_task_index(
    state: &Arc<AppState>,
    task_id: TaskId,
    ws_id: WorkspaceId,
) {
    if let Err(e) = state
        .global_store()
        .upsert_workspace_task_index(task_id, ws_id)
        .await
    {
        tracing::warn!(task_id = %task_id.0, "failed to update task index: {e:?}");
    }
}

pub(super) async fn reload_or_retry_task_for_request(
    store: &Store,
    ws_id: WorkspaceId,
    persisted: PersistedTaskForCreate,
    request: &CreateTaskRequestParts,
) -> Result<PersistedTaskForCreate, CreateTaskApiError> {
    let task = store
        .get_task_with_activity(persisted.task.id)
        .await
        .map_err(internal_store_error)?;
    if let Some(task) = task {
        return Ok(PersistedTaskForCreate {
            task,
            created_in_this_request: persisted.created_in_this_request,
        });
    }
    if persisted.created_in_this_request {
        return Err(task_not_found());
    }
    let Some(task_id) = request.task_id else {
        return Err(task_not_found());
    };
    let retry = store
        .create_task_with_id_result(
            ws_id,
            task_id,
            request.requested_title.clone(),
            request.requested_description.clone(),
        )
        .await
        .map_err(internal_store_error)?;
    if !retry.created {
        validate_requested_task_identity(&retry.task, ws_id, request)?;
    }
    Ok(PersistedTaskForCreate {
        task: retry.task,
        created_in_this_request: retry.created,
    })
}

fn validate_requested_task_identity(
    task: &Task,
    ws_id: WorkspaceId,
    request: &CreateTaskRequestParts,
) -> Result<(), CreateTaskApiError> {
    if request.task_id.is_none() {
        return Ok(());
    }
    if task.workspace_id != ws_id
        || !task_request_matches(
            task,
            &request.requested_title,
            &request.requested_description,
        )
    {
        return Err(task_id_conflict());
    }
    Ok(())
}

fn internal_store_error(error: impl std::fmt::Display) -> CreateTaskApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiErrorResp {
            error: logs::redact_sensitive(&error.to_string()),
        }),
    )
}

fn task_id_conflict() -> CreateTaskApiError {
    (
        StatusCode::CONFLICT,
        Json(ApiErrorResp {
            error: "task id already exists".to_string(),
        }),
    )
}

fn task_not_found() -> CreateTaskApiError {
    (
        StatusCode::NOT_FOUND,
        Json(ApiErrorResp {
            error: "task not found".to_string(),
        }),
    )
}
