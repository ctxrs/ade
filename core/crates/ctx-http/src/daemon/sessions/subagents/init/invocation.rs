use std::sync::Arc;

use ctx_core::ids::TurnId;
use ctx_core::models::{Session, SessionTurnStatus, SubagentInvocation};

use crate::daemon::AppState;

use super::super::{emit_subagent_invocation_notice, internal_api_error, ApiResult};

pub(super) struct StartedSubagentInvocation {
    pub(super) invocation_id: String,
    pub(super) tool_call_id: String,
    pub(super) parent_turn_id: Option<TurnId>,
}

pub(super) async fn start_subagent_invocation(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    parent: &Session,
    requested_count: usize,
    request_json: Option<serde_json::Value>,
    requested_tool_call_id: Option<&str>,
) -> ApiResult<StartedSubagentInvocation> {
    let mut requested_tool_call_id = requested_tool_call_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let invocation_id = requested_tool_call_id
        .clone()
        .unwrap_or_else(|| format!("subagent-{}", uuid::Uuid::new_v4()));
    let tool_call_id = requested_tool_call_id
        .take()
        .unwrap_or_else(|| invocation_id.clone());
    let parent_turn_id = resolve_parent_turn_id(store, parent, &tool_call_id).await;

    let now = chrono::Utc::now();
    let invocation = SubagentInvocation {
        id: invocation_id.clone(),
        tool_call_id: tool_call_id.clone(),
        parent_session_id: parent.id,
        parent_turn_id,
        requested_count: requested_count as i64,
        request_json,
        status: "requested".to_string(),
        created_at: now,
        updated_at: now,
        children: Vec::new(),
    };
    store
        .upsert_subagent_invocation(invocation)
        .await
        .map_err(internal_api_error)?;
    emit_subagent_invocation_notice(
        state,
        parent.id,
        parent_turn_id,
        serde_json::json!({
            "kind": "subagent_invocation_created",
            "invocation_id": invocation_id.clone(),
            "tool_call_id": tool_call_id.clone(),
            "status": "requested",
            "requested_count": requested_count,
            "child_session_ids": Vec::<String>::new(),
        }),
    )
    .await?;

    let running_at = chrono::Utc::now();
    store
        .update_subagent_invocation_status(&invocation_id, "running", running_at)
        .await
        .map_err(internal_api_error)?;
    emit_subagent_invocation_notice(
        state,
        parent.id,
        parent_turn_id,
        serde_json::json!({
            "kind": "subagent_invocation_updated",
            "invocation_id": invocation_id.clone(),
            "tool_call_id": tool_call_id.clone(),
            "status": "running",
            "child_session_ids": Vec::<String>::new(),
        }),
    )
    .await?;

    Ok(StartedSubagentInvocation {
        invocation_id,
        tool_call_id,
        parent_turn_id,
    })
}

async fn resolve_parent_turn_id(
    store: &ctx_store::Store,
    parent: &Session,
    tool_call_id: &str,
) -> Option<TurnId> {
    if !tool_call_id.trim().is_empty() {
        if let Ok(Some(tool)) = store.get_session_turn_tool(parent.id, tool_call_id).await {
            return Some(tool.turn_id);
        }
    }
    let Ok(turns) = store
        .list_session_turns_page_by_seq(parent.id, None, Some(5))
        .await
    else {
        return None;
    };
    turns.iter().rev().find_map(|turn| {
        matches!(
            turn.status,
            SessionTurnStatus::Starting | SessionTurnStatus::Running | SessionTurnStatus::Queued
        )
        .then_some(turn.turn_id)
    })
}

pub(super) async fn mark_subagent_invocation_failed(
    state: &Arc<AppState>,
    parent: &Session,
    invocation_id: &str,
    tool_call_id: &str,
    parent_turn_id: Option<TurnId>,
    child_session_ids: &[String],
) {
    let updated_at = chrono::Utc::now();
    if let Ok(store) = state.store_for_session(parent.id).await {
        if let Err(update_error) = store
            .update_subagent_invocation_status(invocation_id, "failed", updated_at)
            .await
        {
            tracing::warn!(
                error = ?update_error,
                "failed to update subagent invocation status"
            );
        }
    }
    let _ = emit_subagent_invocation_notice(
        state,
        parent.id,
        parent_turn_id,
        serde_json::json!({
            "kind": "subagent_invocation_updated",
            "invocation_id": invocation_id,
            "tool_call_id": tool_call_id,
            "status": "failed",
            "child_session_ids": child_session_ids,
        }),
    )
    .await;
}
