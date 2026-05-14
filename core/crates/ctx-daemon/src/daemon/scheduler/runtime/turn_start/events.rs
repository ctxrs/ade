use std::sync::Arc;

use ctx_core::ids::{RunId, TurnId};
use ctx_core::models::Session;
use ctx_observability::ops_events::OpsEvent;
use serde_json::json;

use crate::daemon::DaemonState;

pub(super) struct ProviderRunStartedEvent<'a> {
    pub(super) state: &'a Arc<DaemonState>,
    pub(super) session: &'a Session,
    pub(super) run_id: RunId,
    pub(super) turn_id: TurnId,
    pub(super) workdir_str: &'a str,
    pub(super) full_model_id: &'a str,
    pub(super) execution_environment: &'a str,
    pub(super) session_root_kind: &'a str,
}

pub(super) fn emit_provider_run_started_event(event: ProviderRunStartedEvent<'_>) {
    let ProviderRunStartedEvent {
        state,
        session,
        run_id,
        turn_id,
        workdir_str,
        full_model_id,
        execution_environment,
        session_root_kind,
    } = event;
    let mut run_event = OpsEvent::new("info", "provider_run_started");
    run_event.session_id = Some(session.id.0.to_string());
    run_event.worktree_id = Some(session.worktree_id.0.to_string());
    run_event.run_id = Some(run_id.0.to_string());
    run_event.turn_id = Some(turn_id.0.to_string());
    run_event.provider_id = Some(session.provider_id.clone());
    run_event.cwd = Some(workdir_str.to_string());
    run_event.worktree_root = Some(workdir_str.to_string());
    run_event.meta = Some(json!({
        "model_id": full_model_id,
        "reasoning_effort": session.reasoning_effort.clone(),
        "execution_environment": execution_environment,
        "session_root_kind": session_root_kind,
    }));
    state.telemetry.ops_events.emit(run_event);
}
