use ctx_store::Store;

use super::*;
use crate::api::sessions::titles_and_modes::model::error::{
    internal_session_model_error, session_model_error, SessionModelResult,
};
use crate::api::sessions::titles_and_modes::model::resolve::ResolvedSessionModelUpdate;

pub(super) async fn persist_session_model_update(
    state: &Arc<AppState>,
    store: &Store,
    session_id: SessionId,
    resolved_model: &ResolvedSessionModelUpdate,
) -> SessionModelResult<Session> {
    store
        .update_session_model_config(
            session_id,
            resolved_model.model_id.clone(),
            resolved_model.reasoning_effort.clone(),
        )
        .await
        .map_err(internal_session_model_error)?;

    let updated = store
        .get_session(session_id)
        .await
        .map_err(internal_session_model_error)?
        .ok_or_else(|| session_model_error(StatusCode::NOT_FOUND, "session not found"))?;
    state.remember_session_meta(&updated).await;

    let event = store
        .append_session_event(
            session_id,
            None,
            None,
            SessionEventType::Init,
            serde_json::json!({
                "current_model_id": resolved_model.full_model_id,
                "reasoning_effort": resolved_model.reasoning_effort.clone(),
            }),
        )
        .await
        .map_err(internal_session_model_error)?;
    state.publish_event(event).await;

    if let Err(error) = crate::daemon::workspaces::update_workspace_provider_preferred_model_id(
        state,
        updated.workspace_id,
        &updated.provider_id,
        Some(resolved_model.full_model_id.clone()),
    )
    .await
    {
        tracing::warn!(
            session_id = %updated.id.0,
            workspace_id = %updated.workspace_id.0,
            provider_id = updated.provider_id.as_str(),
            "failed to persist workspace provider model preference after session model update: {error:#}"
        );
    }

    if let Err(e) = state.emit_workspace_task_upsert(updated.task_id).await {
        tracing::warn!(
            task_id = %updated.task_id.0,
            "workspace active snapshot refresh failed after session model update: {e:?}"
        );
    }

    Ok(updated)
}
