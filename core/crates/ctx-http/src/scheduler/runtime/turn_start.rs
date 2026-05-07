use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;

use crate::daemon::AppState;
use crate::ops_events::OpsEvent;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
use crate::scheduler::QueuedMessage;
use crate::settings::ProviderControlMode;
use ctx_core::ids::{MessageId, RunId, TurnId};
use ctx_core::models::{
    ExecutionEnvironment, Message, MessageDelivery, Session, SessionTurnStatus,
};
use ctx_core::provider_policy::{CTX_CRP_LAUNCH_POLICY_ENV, CTX_CRP_LAUNCH_POLICY_FULL};
use serde_json::json;

use super::helpers::compute_context_window_metrics;

pub(super) fn apply_crp_launch_policy_env_for_control_mode(
    provider_env: &mut std::collections::HashMap<String, String>,
    control_mode: &ProviderControlMode,
) {
    provider_env.remove(CTX_CRP_LAUNCH_POLICY_ENV);
    match control_mode {
        ProviderControlMode::Full => {
            provider_env.insert(
                CTX_CRP_LAUNCH_POLICY_ENV.to_string(),
                CTX_CRP_LAUNCH_POLICY_FULL.to_string(),
            );
        }
        ProviderControlMode::HarnessNative | ProviderControlMode::CtxEnforced => {}
    }
}

const DEFAULT_TURN_START_DEADLINE: Duration = Duration::from_secs(60);
const CONTAINER_TURN_START_DEADLINE: Duration = Duration::from_secs(135);

pub(super) fn turn_start_deadline(
    provider_env: &std::collections::HashMap<String, String>,
) -> Duration {
    turn_start_deadline_with_override(
        provider_env,
        std::env::var("CTX_TURN_START_DEADLINE_MS").ok().as_deref(),
    )
}

fn turn_start_deadline_with_override(
    provider_env: &std::collections::HashMap<String, String>,
    configured: Option<&str>,
) -> Duration {
    configured
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| {
            if provider_env.contains_key("CTX_HARNESS_CONTAINER_ID") {
                CONTAINER_TURN_START_DEADLINE
            } else {
                DEFAULT_TURN_START_DEADLINE
            }
        })
}

pub(super) async fn record_queue_wait_metric(
    state: &Arc<AppState>,
    session: &Session,
    full_model_id: &str,
    execution_environment: &str,
    session_root_kind: &str,
    perf_run_id: Option<String>,
    queue_wait_ms: u64,
) {
    let mut queue_labels = HashMap::new();
    queue_labels.insert("provider_id".to_string(), session.provider_id.clone());
    queue_labels.insert("model_id".to_string(), full_model_id.to_string());
    queue_labels.insert(
        "execution_environment".to_string(),
        execution_environment.to_string(),
    );
    queue_labels.insert(
        "session_root_kind".to_string(),
        session_root_kind.to_string(),
    );
    queue_labels.insert("event".to_string(), "queue_wait".to_string());
    let queue_metric = PerfMetric {
        name: "scheduler.queue_wait_ms".to_string(),
        kind: PerfMetricKind::Histogram,
        unit: "ms".to_string(),
        value: queue_wait_ms as f64,
        labels: queue_labels,
    };
    state
        .telemetry
        .perf_telemetry
        .record_metric(queue_metric, perf_run_id, None, None)
        .await;
}

pub(super) struct PreparedTurnStart {
    pub(super) message: Message,
    pub(super) message_id: MessageId,
    pub(super) perf_run_id: Option<String>,
    pub(super) run_id: RunId,
    pub(super) turn_id: TurnId,
    pub(super) provider_session_ref: Option<String>,
    pub(super) context_window_metrics: Option<serde_json::Value>,
}

pub(super) async fn prepare_turn_start(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    session: &Session,
    workdir_str: &str,
    full_model_id: &str,
    execution_environment: ExecutionEnvironment,
    session_root_kind: &str,
    queued: QueuedMessage,
) -> Result<PreparedTurnStart> {
    let QueuedMessage {
        mut message,
        enqueued_at,
        run_id: perf_run_id,
    } = queued;
    let message_id = message.id;
    let queue_wait_ms = enqueued_at.elapsed().as_millis() as u64;
    record_queue_wait_metric(
        state,
        session,
        full_model_id,
        execution_environment.as_str(),
        session_root_kind,
        perf_run_id.clone(),
        queue_wait_ms,
    )
    .await;
    let run_id = message.run_id.get_or_insert_with(RunId::new).to_owned();
    let turn_id = message.turn_id.get_or_insert_with(TurnId::new).to_owned();

    emit_provider_run_started_event(ProviderRunStartedEvent {
        state,
        session,
        run_id,
        turn_id,
        workdir_str,
        full_model_id,
        execution_environment: execution_environment.as_str(),
        session_root_kind,
    });

    if message.delivered_at.is_none() {
        store.mark_message_delivered(message.id).await?;
        message.delivery = MessageDelivery::Immediate;
        message.delivered_at = Some(Utc::now());
    }
    store
        .update_session_turn_status(
            session.id,
            turn_id,
            SessionTurnStatus::Starting,
            None,
            None,
            Utc::now(),
        )
        .await?;

    let context_window_metrics =
        compute_context_window_metrics(&session.provider_id, full_model_id, &message.content);

    Ok(PreparedTurnStart {
        message,
        message_id,
        perf_run_id,
        run_id,
        turn_id,
        provider_session_ref: session.provider_session_ref.clone(),
        context_window_metrics,
    })
}

pub(super) struct ProviderRunStartedEvent<'a> {
    pub(super) state: &'a Arc<AppState>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_start_deadline_uses_longer_container_budget() {
        let env = HashMap::from([(
            "CTX_HARNESS_CONTAINER_ID".to_string(),
            "ctx-harness-test".to_string(),
        )]);

        assert_eq!(
            turn_start_deadline_with_override(&env, None),
            CONTAINER_TURN_START_DEADLINE
        );
    }

    #[test]
    fn turn_start_deadline_env_override_still_wins() {
        let env = HashMap::from([(
            "CTX_HARNESS_CONTAINER_ID".to_string(),
            "ctx-harness-test".to_string(),
        )]);

        assert_eq!(
            turn_start_deadline_with_override(&env, Some("2500")),
            Duration::from_millis(2500)
        );
    }
}
