use super::*;

#[derive(Debug, Deserialize)]
pub(crate) struct SetSessionModeReq {
    pub(crate) mode_id: String,
}

pub(crate) async fn set_session_mode(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SetSessionModeReq>,
) -> Result<StatusCode, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let store = store_for_existing_session_status_for_write(&state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let workspace = state
        .global_store()
        .get_workspace(session.workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let resolved_worktree = crate::daemon::workspaces::resolve_existing_worktree_execution(
        &state,
        &store,
        &workspace,
        worktree.id,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let execution_environment = resolved_worktree.execution_environment();
    if session.execution_environment != execution_environment {
        tracing::warn!(
            session_id = %session.id.0,
            stored = session.execution_environment.as_str(),
            resolved = execution_environment.as_str(),
            "session mode update resolved a different execution_environment than persisted metadata"
        );
    }
    let install_target =
        crate::daemon::execution_effective::effective_install_target_for_environment(
            state.as_ref(),
            worktree.workspace_id,
            execution_environment,
        )
        .await
        .map_err(|err| {
            tracing::warn!(
                workspace_id = %worktree.workspace_id.0,
                "set_session_mode failed to load execution settings: {err:#}",
            );
            crate::api::shared::status_code_for_internal_error(&err)
        })?;

    let adapter =
        ctx_provider_runtime::provider_launch::resolver::ensure_provider_adapter_for_target(
            state.as_ref(),
            &session.provider_id,
            install_target,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    adapter
        .set_session_mode(session.id.0.to_string(), req.mode_id.clone())
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let event = store
        .append_session_event(
            session_id,
            None,
            None,
            SessionEventType::Init,
            serde_json::json!({"set_mode": req.mode_id}),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state.publish_event(event).await;

    Ok(StatusCode::OK)
}
