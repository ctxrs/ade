use super::*;

#[path = "default_session_flow/effects.rs"]
mod effects;

use effects::{emit_task_upsert, rollback_new_task_after_default_session_failure};

pub(super) async fn ensure_default_session_for_task(
    handles: &TaskApiHandles,
    store: Store,
    workspace: Workspace,
    task: Task,
    requested_default_session: Option<CreateTaskDefaultSessionReq>,
    default_session_plan: Option<(ExecutionEnvironment, String, String, Option<String>)>,
    created_task_in_this_request: bool,
) -> Result<Task, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(primary_session_id) = task.primary_session_id {
        if let Some(default_session_req) = requested_default_session {
            if let Err(status) =
                super::super::session_creation::replay_requested_default_session_for_task(
                    handles,
                    store.clone(),
                    task.clone(),
                    workspace.clone(),
                    default_session_req,
                    primary_session_id,
                )
                .await
            {
                return Err((
                    status,
                    Json(ApiErrorResp {
                        error: "task id already exists with a different default session"
                            .to_string(),
                    }),
                ));
            }
        }
        emit_task_upsert(handles, task.id).await;
        return Ok(task);
    }

    let default_session_result = if let Some(default_session_req) = requested_default_session {
        super::super::session_creation::create_requested_default_session_for_task(
            handles,
            store.clone(),
            task.clone(),
            workspace.clone(),
            default_session_req,
        )
        .await
    } else {
        let (execution_environment, provider_id, model_id, reasoning_effort) =
            match default_session_plan {
                Some(plan) => plan,
                None => {
                    match preflight_default_session_creation(handles, &store, &workspace).await {
                        Ok(plan) => plan,
                        Err(err) => {
                            if created_task_in_this_request {
                                rollback_new_task_after_default_session_failure(
                                    handles, &store, &workspace, task.id,
                                )
                                .await;
                            }
                            return Err(err);
                        }
                    }
                }
            };
        handles
            .tasks
            .create_session_for_loaded_task(
                store.clone(),
                task.clone(),
                workspace.clone(),
                crate::daemon::tasks::CreateTaskSessionInput::from_default_seed(
                    crate::daemon::tasks::DefaultSessionSeed {
                        provider_id,
                        model_id,
                        reasoning_effort,
                        execution_environment,
                    },
                ),
            )
            .await
            .map_err(task_session_create_status)
    };
    if let Err(status) = default_session_result {
        if created_task_in_this_request {
            rollback_new_task_after_default_session_failure(handles, &store, &workspace, task.id)
                .await;
        }
        return Err((
            status,
            Json(ApiErrorResp {
                error: "failed to create default session".to_string(),
            }),
        ));
    }

    let task = match store.get_task_with_activity(task.id).await.map_err(|e| {
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

    emit_task_upsert(handles, task.id).await;
    Ok(task)
}
