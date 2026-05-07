use super::*;
use crate::api::sessions;
use crate::api::shared;
use ctx_session_service::session_creation::{
    session_matches_creation_identity, validate_create_session_request, CreateSessionRequestError,
    CreateSessionRequestPolicy, SessionCreationIdentity,
};
use ctx_session_tools::model_resolution::{compose_model_id, resolve_model_id};

#[path = "creation_session/cleanup.rs"]
mod cleanup;
#[path = "creation_session/existing.rs"]
mod existing;
#[path = "creation_session/initial_prompt.rs"]
mod initial_prompt;
#[path = "creation_session/replay.rs"]
mod replay;
#[path = "creation_session/request.rs"]
mod request;
#[path = "creation_session/worktree.rs"]
mod worktree;

use cleanup::cleanup_orphaned_provisioned_worktree;
use existing::resolve_existing_requested_session;
use initial_prompt::{seed_initial_prompt, InitialPromptSeed};
pub(super) use replay::{
    create_requested_default_session_for_task, replay_requested_default_session_for_task,
};
pub(in crate::api) use request::CreateSessionReq;
pub(in crate::api::tasks) use request::CreateTaskDefaultSessionReq;
use worktree::resolve_session_worktree_for_task;

async fn create_session_for_task_inner(
    state: Arc<AppState>,
    task_id: TaskId,
    headers: HeaderMap,
    req: CreateSessionReq,
) -> Result<Json<Session>, StatusCode> {
    let store = state
        .store_for_task(task_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let task = store
        .get_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let workspace = state
        .global_store()
        .get_workspace(task.workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    create_session_for_loaded_task_inner(state, store, task, workspace, headers, req).await
}

async fn create_session_for_loaded_task_inner(
    state: Arc<AppState>,
    store: Store,
    task: Task,
    workspace: Workspace,
    headers: HeaderMap,
    req: CreateSessionReq,
) -> Result<Json<Session>, StatusCode> {
    let task_id = task.id;
    let run_id_header = headers
        .get("x-ctx-run-id")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());
    let provider_id = req.provider_id.trim().to_string();
    if !state
        .providers
        .adapters
        .lock()
        .await
        .contains_key(&provider_id)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let session_request = match validate_create_session_request(CreateSessionRequestPolicy {
        requested_session_id: req.id.as_deref(),
        parent_session_id: req.parent_session_id.as_deref(),
        relationship: req.relationship.as_deref(),
        initial_prompt_present: req.initial_prompt.is_some(),
        initial_message_id_present: req.initial_message_id.is_some(),
        initial_turn_id_present: req.initial_turn_id.is_some(),
        task_primary_session_id: task.primary_session_id,
    }) {
        Ok(decision) => decision,
        Err(CreateSessionRequestError::MissingInitialPromptIds) => {
            state
                .emit_compat_payload_reject_counter(
                    "tasks.create_session",
                    "missing_initial_ids",
                    None,
                )
                .await;
            return Err(StatusCode::BAD_REQUEST);
        }
        Err(CreateSessionRequestError::PrimarySessionConflict) => {
            return Err(StatusCode::CONFLICT);
        }
        Err(
            CreateSessionRequestError::InvalidSessionId
            | CreateSessionRequestError::InvalidParentSessionId
            | CreateSessionRequestError::RelationshipRequiresParent,
        ) => {
            return Err(StatusCode::BAD_REQUEST);
        }
    };
    let session_id = session_request.session_id;
    let parent_session_id = session_request.parent_session_id;
    let relationship = session_request.relationship;
    let requested_relationship = relationship.clone();

    let worktree_resolution = resolve_session_worktree_for_task(
        &state,
        &store,
        &task,
        &workspace,
        req.worktree_id.as_deref(),
        req.execution_environment,
    )
    .await?;
    let worktree_id = worktree_resolution.worktree_id;
    let created_worktree_id = worktree_resolution.created_worktree_id;
    let execution_environment = worktree_resolution.execution_environment;
    let catalog = match sessions::load_provider_model_catalog_for_execution_environment(
        &state,
        &workspace,
        &provider_id,
        execution_environment,
    )
    .await
    {
        Ok(catalog) => catalog,
        Err(error) => {
            tracing::warn!(
                workspace_id = %workspace.id.0,
                provider_id = provider_id,
                execution_environment = execution_environment.as_str(),
                "failed to load provider model catalog while creating session: {error}"
            );
            if let Some(created_worktree_id) = created_worktree_id {
                cleanup_orphaned_provisioned_worktree(
                    &state,
                    &store,
                    &workspace,
                    task_id,
                    created_worktree_id,
                )
                .await;
            }
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let resolved_model = match resolve_model_id(
        Some(req.model_id.as_str()),
        req.reasoning_effort.as_deref(),
        None,
        catalog.as_ref(),
    ) {
        Ok(model) => model,
        Err(_) => {
            if let Some(created_worktree_id) = created_worktree_id {
                cleanup_orphaned_provisioned_worktree(
                    &state,
                    &store,
                    &workspace,
                    task_id,
                    created_worktree_id,
                )
                .await;
            }
            return Err(StatusCode::BAD_REQUEST);
        }
    };
    let model_id = resolved_model.model_id.clone();
    let reasoning_effort = resolved_model.reasoning_effort.clone();
    let preferred_model_id = compose_model_id(&model_id, reasoning_effort.as_deref());

    if let Some(existing) = resolve_existing_requested_session(
        &state,
        &store,
        &task,
        &workspace,
        session_id,
        created_worktree_id,
        worktree_id,
        execution_environment,
        &provider_id,
        &model_id,
        reasoning_effort.as_deref(),
        parent_session_id,
        relationship.as_deref(),
        req.remember_model_preference,
        &preferred_model_id,
    )
    .await?
    {
        return Ok(Json(existing));
    }

    if let Ok(Some(worktree)) = store.get_worktree(worktree_id).await {
        if let Err(e) =
            vcs_hooks::ensure_task_commit_hook(&state, &workspace, &worktree, task.id).await
        {
            tracing::warn!(
                task_id = %task.id.0,
                worktree_id = %worktree.id.0,
                "failed to configure vcs hooks: {e:#}"
            );
        }
    }

    let requested_session_id = session_id;
    let session = if let Some(session_id) = requested_session_id {
        match store
            .create_session_with_id_and_reasoning_effort(
                session_id,
                task_id,
                task.workspace_id,
                worktree_id,
                execution_environment,
                provider_id.clone(),
                model_id.clone(),
                reasoning_effort.clone(),
                "implementer".to_string(),
                parent_session_id,
                relationship.clone(),
                None,
            )
            .await
        {
            Ok(session) => session,
            Err(_) => {
                if let Some(created_worktree_id) = created_worktree_id {
                    cleanup_orphaned_provisioned_worktree(
                        &state,
                        &store,
                        &workspace,
                        task_id,
                        created_worktree_id,
                    )
                    .await;
                }
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    } else {
        match store
            .create_session_with_reasoning_effort(
                task_id,
                task.workspace_id,
                worktree_id,
                execution_environment,
                provider_id.clone(),
                model_id.clone(),
                reasoning_effort.clone(),
                "implementer".to_string(),
                parent_session_id,
                relationship.clone(),
                None,
            )
            .await
        {
            Ok(session) => session,
            Err(_) => {
                if let Some(created_worktree_id) = created_worktree_id {
                    cleanup_orphaned_provisioned_worktree(
                        &state,
                        &store,
                        &workspace,
                        task_id,
                        created_worktree_id,
                    )
                    .await;
                }
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    };
    if let Some(session_id) = requested_session_id {
        if session.id != session_id
            || !session_matches_creation_identity(
                &session,
                SessionCreationIdentity {
                    task_id,
                    workspace_id: task.workspace_id,
                    worktree_id,
                    execution_environment,
                    provider_id: &provider_id,
                    model_id: &model_id,
                    reasoning_effort: reasoning_effort.as_deref(),
                    parent_session_id,
                    relationship: requested_relationship.as_deref(),
                },
            )
        {
            return Err(StatusCode::CONFLICT);
        }
    }
    state.sessions.remember_session_meta(&session).await;
    if let Err(e) = retry_global_index_write(|| async {
        state
            .global_store()
            .upsert_workspace_session_index(session.id, task.workspace_id)
            .await
    })
    .await
    {
        tracing::warn!(session_id = %session.id.0, "failed to update session index: {e:?}");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    if session.parent_session_id.is_none() && session.relationship.is_none() {
        let _ = store
            .set_task_primary_session(task.id, session.id, worktree_id)
            .await;
    }

    seed_initial_prompt(
        &state,
        &store,
        &session,
        InitialPromptSeed {
            prompt: req.initial_prompt,
            message_id: req.initial_message_id,
            turn_id: req.initial_turn_id,
            run_id_header: run_id_header.clone(),
        },
    )
    .await?;

    if req.remember_model_preference {
        if let Err(error) =
            crate::workspace_provider_model_preferences::update_workspace_provider_preferred_model_id(
                &state,
                task.workspace_id,
                &provider_id,
                Some(preferred_model_id),
            )
            .await
        {
            tracing::warn!(
                session_id = %session.id.0,
                workspace_id = %task.workspace_id.0,
                provider_id = provider_id,
                "failed to persist workspace provider model preference: {error:#}"
            );
        }
    }

    let worktree = match state.store_for_session(session.id).await {
        Ok(store) => store.get_worktree(session.worktree_id).await.ok().flatten(),
        Err(_) => None,
    };
    let session_root_kind = session_root_kind_for_worktree(worktree.as_ref()).to_string();
    state
        .telemetry
        .telemetry
        .emit(TelemetryEvent::session_started(
            session.provider_id.clone(),
            compose_model_id(&session.model_id, session.reasoning_effort.as_deref()),
            Some(session.execution_environment.as_str().to_string()),
            Some(session_root_kind.clone()),
        ))
        .await;
    let mut ops_event = OpsEvent::new("info", "session_started");
    ops_event.session_id = Some(session.id.0.to_string());
    ops_event.worktree_id = Some(session.worktree_id.0.to_string());
    ops_event.provider_id = Some(session.provider_id.clone());
    ops_event.meta = Some(serde_json::json!({
        "model_id": compose_model_id(&session.model_id, session.reasoning_effort.as_deref()),
        "reasoning_effort": session.reasoning_effort.clone(),
        "execution_environment": session.execution_environment.as_str(),
        "session_root_kind": session_root_kind.clone(),
        "parent_session_id": session.parent_session_id.map(|id| id.0.to_string()),
        "relationship": session.relationship.clone(),
    }));
    state.telemetry.ops_events.emit(ops_event);
    if let Err(e) = state.emit_workspace_task_upsert(session.task_id).await {
        tracing::warn!(task_id = %session.task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }

    Ok(Json(session))
}

pub(in crate::api) async fn create_session_for_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CreateSessionReq>,
) -> Result<Json<Session>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let creation_lock = state.sessions.task_session_creation_lock(task_id).await;
    let _creation_guard = creation_lock.lock().await;
    create_session_for_task_inner(state, task_id, headers, req).await
}

pub(in crate::api) struct DefaultSessionSeed {
    pub provider_id: String,
    pub model_id: String,
    pub reasoning_effort: Option<String>,
    pub execution_environment: ExecutionEnvironment,
}

pub(in crate::api) async fn create_default_session_for_task(
    state: Arc<AppState>,
    store: Store,
    task: Task,
    workspace: Workspace,
    seed: DefaultSessionSeed,
) -> Result<Session, StatusCode> {
    let Json(session) = create_session_for_loaded_task_inner(
        state,
        store,
        task,
        workspace,
        HeaderMap::new(),
        CreateSessionReq {
            id: None,
            provider_id: seed.provider_id,
            model_id: seed.model_id,
            reasoning_effort: seed.reasoning_effort,
            remember_model_preference: false,
            parent_session_id: None,
            relationship: None,
            initial_prompt: None,
            initial_message_id: None,
            initial_turn_id: None,
            worktree_id: None,
            execution_environment: Some(seed.execution_environment),
        },
    )
    .await?;
    Ok(session)
}
