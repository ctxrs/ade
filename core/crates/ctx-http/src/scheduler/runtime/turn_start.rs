use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use crate::daemon::AppState;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
use crate::settings::ProviderControlMode;
use ctx_core::ids::{MessageId, RunId, TurnId};
use ctx_core::models::Session;
use ctx_core::provider_ids::CODEX_PROVIDER_ID;
use ctx_core::provider_policy::{CTX_CRP_LAUNCH_POLICY_ENV, CTX_CRP_LAUNCH_POLICY_FULL};

use super::super::terminal::{finalize_failed_turn, FailedTurnTerminalization};

pub(super) fn provider_mode_id_for(
    provider_id: &str,
    control_mode: &ProviderControlMode,
) -> Option<&'static str> {
    match control_mode {
        ProviderControlMode::Full => match provider_id {
            CODEX_PROVIDER_ID => Some("full-access"),
            "claude-crp" => Some("bypassPermissions"),
            "droid" => Some("auto_high"),
            _ => None,
        },
        ProviderControlMode::HarnessNative | ProviderControlMode::CtxEnforced => None,
    }
}

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

pub(super) fn turn_start_deadline() -> Duration {
    std::env::var("CTX_TURN_START_DEADLINE_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_TURN_START_DEADLINE)
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

pub(super) async fn emit_turn_start_failed(
    state: &std::sync::Arc<AppState>,
    session: &Session,
    run_id: RunId,
    turn_id: TurnId,
    message_id: MessageId,
    err: &anyhow::Error,
) {
    let error_message = err.to_string();
    let _ = finalize_failed_turn(
        state,
        session.id,
        Some(run_id),
        turn_id,
        message_id,
        FailedTurnTerminalization {
            message: &error_message,
            reason: Some("start_failed"),
            details: None,
            kind: Some(json!("start_failed")),
            emit_error_event: true,
        },
    )
    .await;
}
