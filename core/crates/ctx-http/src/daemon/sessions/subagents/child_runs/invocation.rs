use std::sync::{Arc, Weak};

use ctx_core::ids::{SessionId, TurnId, WorktreeId};
use ctx_core::models::{SessionEventType, SubagentInvocationChild};
use ctx_observability::logs;

use crate::daemon::AppState;

use super::super::errors::{internal_api_error, store_for_session, ApiResult};
use super::status::subagent_status_from_turn_status;
use super::wait::wait_for_run_terminal_turn;

pub(in crate::daemon::sessions::subagents) async fn emit_subagent_invocation_notice(
    state: &Arc<AppState>,
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
    state_weak: &Weak<AppState>,
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

pub(in crate::daemon::sessions::subagents) async fn finalize_subagent_invocation(
    state: &Arc<AppState>,
    invocation_id: &str,
    tool_call_id: &str,
    parent_session_id: SessionId,
    parent_turn_id: Option<TurnId>,
) -> Result<(), String> {
    let store = state
        .store_for_session(parent_session_id)
        .await
        .map_err(|error| logs::redact_sensitive(&error.to_string()))?;
    let Some(invocation) = store
        .get_subagent_invocation(invocation_id)
        .await
        .map_err(|error| logs::redact_sensitive(&error.to_string()))?
    else {
        return Ok(());
    };

    if invocation.children.is_empty() {
        return Ok(());
    }
    if invocation
        .children
        .iter()
        .any(|child| child.status == "running")
    {
        return Ok(());
    }

    let final_status = if invocation
        .children
        .iter()
        .all(|child| child.status == "completed")
    {
        "completed"
    } else {
        "failed"
    };
    if invocation.status == final_status {
        return Ok(());
    }

    let updated_at = chrono::Utc::now();
    store
        .update_subagent_invocation_status(invocation_id, final_status, updated_at)
        .await
        .map_err(|error| logs::redact_sensitive(&error.to_string()))?;
    let child_session_ids = invocation
        .children
        .iter()
        .map(|child| child.child_session_id.0.to_string())
        .collect::<Vec<_>>();
    let child_statuses = invocation
        .children
        .iter()
        .map(|child| {
            serde_json::json!({
                "session_id": child.child_session_id.0.to_string(),
                "status": child.status,
            })
        })
        .collect::<Vec<_>>();
    emit_subagent_invocation_notice(
        state,
        parent_session_id,
        parent_turn_id,
        serde_json::json!({
            "kind": "subagent_invocation_updated",
            "invocation_id": invocation_id,
            "tool_call_id": tool_call_id,
            "status": final_status,
            "child_session_ids": child_session_ids,
            "child_statuses": child_statuses,
        }),
    )
    .await
    .map_err(|error| error.message().to_string())?;

    Ok(())
}
