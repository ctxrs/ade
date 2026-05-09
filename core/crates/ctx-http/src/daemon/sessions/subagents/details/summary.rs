use std::time::Duration;

use ctx_core::ids::SessionId;
use ctx_core::models::{MessageDelivery, SessionTurn, SessionTurnStatus};

use super::super::{api_error, internal_api_error, ApiResult, SubagentErrorKind};
use super::refs::{decode_agent_ref, encode_agent_ref, encode_run_ref};
use crate::api::sessions::AgentSummary;

pub(super) fn agent_terminal_result_status(status: SessionTurnStatus) -> Option<&'static str> {
    match status {
        SessionTurnStatus::Completed => Some("completed"),
        SessionTurnStatus::Interrupted => Some("interrupted"),
        SessionTurnStatus::Failed => Some("failed"),
        SessionTurnStatus::Queued | SessionTurnStatus::Starting | SessionTurnStatus::Running => {
            None
        }
    }
}

pub(in crate::daemon::sessions::subagents) fn agent_delivery_label(
    delivery: &MessageDelivery,
) -> &'static str {
    match delivery {
        MessageDelivery::Immediate => "immediate",
        MessageDelivery::Queued => "queued",
    }
}

pub(in crate::daemon::sessions::subagents) fn is_active_turn_status(
    status: &SessionTurnStatus,
) -> bool {
    matches!(
        status,
        SessionTurnStatus::Queued | SessionTurnStatus::Starting | SessionTurnStatus::Running
    )
}

fn is_terminal_turn_status(status: &SessionTurnStatus) -> bool {
    matches!(
        status,
        SessionTurnStatus::Completed | SessionTurnStatus::Interrupted | SessionTurnStatus::Failed
    )
}

pub(super) fn agent_active_state(status: SessionTurnStatus) -> &'static str {
    match status {
        SessionTurnStatus::Queued => "queued",
        SessionTurnStatus::Starting => "starting",
        SessionTurnStatus::Running => "running",
        SessionTurnStatus::Completed
        | SessionTurnStatus::Interrupted
        | SessionTurnStatus::Failed => "waiting_input",
    }
}

fn agent_health(active_turn: Option<&SessionTurn>, inactivity_timeout: Duration) -> &'static str {
    let Some(turn) = active_turn else {
        return "healthy";
    };

    let stalled_after = inactivity_timeout.max(Duration::from_millis(1));
    let slow_after = stalled_after.checked_div(2).unwrap_or(stalled_after);
    let age = chrono::Utc::now()
        .signed_duration_since(turn.updated_at)
        .to_std()
        .unwrap_or_default();
    if age >= stalled_after {
        "stalled"
    } else if !slow_after.is_zero() && age >= slow_after {
        "slow"
    } else {
        "healthy"
    }
}

async fn latest_terminal_turn_for_session(
    store: &ctx_store::Store,
    session_id: SessionId,
    latest_turn: Option<&SessionTurn>,
) -> ApiResult<Option<SessionTurn>> {
    if latest_turn
        .as_ref()
        .is_some_and(|turn| is_terminal_turn_status(&turn.status))
    {
        return Ok(latest_turn.cloned());
    }

    let mut before_seq = latest_turn.as_ref().and_then(|turn| turn.start_seq);
    loop {
        let page = store
            .list_session_turns_page_by_seq(session_id, before_seq, Some(50))
            .await
            .map_err(internal_api_error)?;
        if page.is_empty() {
            return Ok(None);
        }
        if let Some(turn) = page
            .iter()
            .rev()
            .find(|turn| is_terminal_turn_status(&turn.status))
            .cloned()
        {
            return Ok(Some(turn));
        }
        before_seq = page.first().and_then(|turn| turn.start_seq);
        if before_seq.is_none() {
            return Ok(None);
        }
    }
}

pub(in crate::daemon::sessions::subagents) async fn resolve_child_agent_session(
    store: &ctx_store::Store,
    parent: &ctx_core::models::Session,
    raw_agent_id: &str,
) -> ApiResult<ctx_core::models::Session> {
    let agent_id = decode_agent_ref(raw_agent_id)
        .map_err(|error| api_error(SubagentErrorKind::BadRequest, error))?;
    let child = store
        .get_active_subagent_session(parent.id, agent_id)
        .await
        .map_err(internal_api_error)?
        .ok_or_else(|| api_error(SubagentErrorKind::NotFound, "agent not found"))?;
    Ok(child)
}

pub(in crate::daemon::sessions::subagents) async fn build_agent_summary(
    store: &ctx_store::Store,
    session_id: SessionId,
    session_title: &str,
    inactivity_timeout: Duration,
) -> ApiResult<(AgentSummary, Option<SessionTurn>)> {
    let latest_turn = store
        .get_latest_turn_for_session(session_id)
        .await
        .map_err(internal_api_error)?;
    let latest_terminal_turn =
        latest_terminal_turn_for_session(store, session_id, latest_turn.as_ref()).await?;
    let active_turn = match store.get_running_turn_for_session(session_id).await {
        Ok(Some(turn)) => Some(turn),
        Ok(None) => latest_turn
            .clone()
            .filter(|turn| is_active_turn_status(&turn.status)),
        Err(error) => return Err(internal_api_error(error)),
    };
    let task_label = {
        let trimmed = session_title.trim();
        if trimmed.is_empty() {
            format!("agent-{}", session_id.0)
        } else {
            trimmed.to_string()
        }
    };
    let summary = AgentSummary {
        agent_id: encode_agent_ref(session_id),
        task_label,
        state: active_turn
            .as_ref()
            .map(|turn| agent_active_state(turn.status.clone()).to_string())
            .unwrap_or_else(|| "waiting_input".to_string()),
        health: agent_health(active_turn.as_ref(), inactivity_timeout).to_string(),
        current_run_id: active_turn
            .as_ref()
            .and_then(|turn| turn.run_id.map(encode_run_ref)),
        latest_result_status: latest_terminal_turn
            .as_ref()
            .and_then(|turn| agent_terminal_result_status(turn.status.clone()))
            .map(str::to_string),
        last_progress_at: active_turn
            .as_ref()
            .or(latest_turn.as_ref())
            .map(|turn| turn.updated_at.to_rfc3339()),
        last_event_seq: store
            .get_session_last_event_seq(session_id)
            .await
            .map_err(internal_api_error)?,
    };
    Ok((summary, latest_terminal_turn))
}
