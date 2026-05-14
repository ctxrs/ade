use std::time::Duration;

use ctx_core::ids::SessionId;
use ctx_core::models::{MessageDelivery, SessionTurn, SessionTurnStatus};

use super::super::super::{internal_api_error, ApiResult};

pub(in crate::daemon::sessions::subagents::details) fn agent_terminal_result_status(
    status: SessionTurnStatus,
) -> Option<&'static str> {
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

pub(in crate::daemon::sessions::subagents::details) fn agent_active_state(
    status: SessionTurnStatus,
) -> &'static str {
    match status {
        SessionTurnStatus::Queued => "queued",
        SessionTurnStatus::Starting => "starting",
        SessionTurnStatus::Running => "running",
        SessionTurnStatus::Completed
        | SessionTurnStatus::Interrupted
        | SessionTurnStatus::Failed => "waiting_input",
    }
}

pub(super) fn agent_health(
    active_turn: Option<&SessionTurn>,
    inactivity_timeout: Duration,
) -> &'static str {
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

pub(super) async fn latest_terminal_turn_for_session(
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
