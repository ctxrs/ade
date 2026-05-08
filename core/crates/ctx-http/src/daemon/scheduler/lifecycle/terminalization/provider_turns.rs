use super::*;

pub(crate) async fn handle_provider_exit(
    state: &Arc<AppState>,
    session_id: SessionId,
    mut turn: RunningTurn,
) -> bool {
    let run_id = turn.run_id;
    let turn_id = turn.turn_id;
    let message_id = turn.message_id;
    let outcome = wait_for_provider_outcome(
        &mut turn.handle,
        provider_protocol_violation(
            "provider_protocol_violation_missing_outcome",
            "provider exited without reporting a terminal outcome",
        ),
        provider_protocol_violation(
            "provider_protocol_violation_outcome_timeout",
            "provider exited without reporting a terminal outcome before timeout",
        ),
    )
    .await;
    drop(turn.event_tx);
    let finalized = if let Some(events_done) = turn.events_done.take() {
        wait_for_turn_event_loop(session_id, run_id, turn_id, events_done).await;
        finalize_provider_outcome_required(
            state,
            session_id,
            Some(run_id),
            turn_id,
            message_id,
            outcome,
        )
        .await
    } else {
        finalize_provider_outcome_required(
            state,
            session_id,
            Some(run_id),
            turn_id,
            message_id,
            outcome,
        )
        .await
    };
    revoke_turn_mcp_token(state, &mut turn.mcp_token).await;
    finalized
}

pub(crate) async fn handle_provider_stall(
    state: &Arc<AppState>,
    session_id: SessionId,
    mut turn: RunningTurn,
) -> bool {
    let run_id = turn.run_id;
    let turn_id = turn.turn_id;
    let message_id = turn.message_id;
    let _ = turn.adapter.cancel(&mut turn.handle).await;
    let _ = wait_for_provider_outcome(
        &mut turn.handle,
        provider_protocol_violation(
            "provider_protocol_violation_inactivity_timeout",
            "provider stalled without reporting a terminal outcome",
        ),
        provider_protocol_violation(
            "provider_protocol_violation_inactivity_timeout",
            "provider stalled without reporting a terminal outcome before timeout",
        ),
    )
    .await;
    drop(turn.event_tx);
    if let Some(events_done) = turn.events_done.take() {
        wait_for_turn_event_loop(session_id, run_id, turn_id, events_done).await;
    }
    let outcome = provider_protocol_violation(
        "provider_protocol_violation_inactivity_timeout",
        "provider stalled without reporting a terminal outcome before timeout",
    );
    let finalized = finalize_provider_outcome_required(
        state,
        session_id,
        Some(run_id),
        turn_id,
        message_id,
        outcome,
    )
    .await;
    revoke_turn_mcp_token(state, &mut turn.mcp_token).await;
    finalized
}

pub(crate) async fn fail_starting_turn(
    state: &Arc<AppState>,
    session_id: SessionId,
    mut turn: RunningTurn,
    error_message: &str,
) {
    let run_id = turn.run_id;
    let turn_id = turn.turn_id;
    let message_id = turn.message_id;
    let _ = turn.adapter.cancel(&mut turn.handle).await;
    let _ = wait_for_provider_outcome(
        &mut turn.handle,
        provider_protocol_violation("start_not_acknowledged", error_message),
        provider_protocol_violation("start_not_acknowledged", error_message),
    )
    .await;
    drop(turn.event_tx);
    if let Some(events_done) = turn.events_done.take() {
        wait_for_turn_event_loop(session_id, run_id, turn_id, events_done).await;
    }
    let _ = finalize_failed_turn(
        state,
        session_id,
        Some(run_id),
        turn_id,
        message_id,
        FailedTurnTerminalization {
            message: error_message,
            reason: Some("start_not_acknowledged"),
            details: None,
            kind: Some(json!("start_not_acknowledged")),
            emit_error_event: true,
        },
    )
    .await;
    revoke_turn_mcp_token(state, &mut turn.mcp_token).await;
}
