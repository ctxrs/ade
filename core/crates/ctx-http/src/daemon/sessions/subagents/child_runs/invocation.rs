use std::sync::{Arc, Weak};

use ctx_core::ids::{SessionId, TurnId, WorktreeId};
use ctx_core::models::{SessionEventType, SubagentInvocationChild};
use ctx_observability::logs;

use crate::daemon::DaemonState;

use super::super::errors::{internal_api_error, store_for_session, ApiResult};
use super::status::subagent_status_from_turn_status;
use super::wait::wait_for_run_terminal_turn;
pub(in crate::daemon::sessions::subagents) use finalize::finalize_subagent_invocation;

#[path = "invocation/finalize.rs"]
mod finalize;

pub(in crate::daemon::sessions::subagents) async fn emit_subagent_invocation_notice(
    state: &Arc<DaemonState>,
    parent_session_id: SessionId,
    parent_turn_id: Option<TurnId>,
    payload: serde_json::Value,
) -> ApiResult<()> {
    let store = store_for_session(state.as_ref(), parent_session_id).await?;
    let event = store
        .append_session_event(
            parent_session_id,
            None,
            parent_turn_id,
            SessionEventType::Notice,
            payload,
        )
        .await
        .map_err(internal_api_error)?;
    state.publish_event(event).await;
    Ok(())
}

pub(in crate::daemon::sessions::subagents) async fn run_subagent_child(
    state_weak: &Weak<DaemonState>,
    child: SubagentInvocationChild,
    _parent_worktree_id: WorktreeId,
) -> Result<(), String> {
    let run_id = child
        .run_id
        .ok_or_else(|| "subagent run_id missing".to_string())?;
    let Some(state) = state_weak.upgrade() else {
        return Ok(());
    };
    let store = state
        .store_for_session(child.child_session_id)
        .await
        .map_err(|error| logs::redact_sensitive(&error.to_string()))?;
    let status = match wait_for_run_terminal_turn(
        state_weak,
        &store,
        child.child_session_id,
        run_id,
    )
    .await
    {
        Ok(Some(turn)) => subagent_status_from_turn_status(turn.status).to_string(),
        Ok(None) => return Ok(()),
        Err(_) => "unknown".to_string(),
    };

    let child_updated_at = chrono::Utc::now();
    let mut updated_child = child.clone();
    updated_child.status = status.clone();
    updated_child.updated_at = child_updated_at;
    store
        .upsert_subagent_invocation_child(updated_child)
        .await
        .map_err(|error| logs::redact_sensitive(&error.to_string()))?;
    Ok(())
}
