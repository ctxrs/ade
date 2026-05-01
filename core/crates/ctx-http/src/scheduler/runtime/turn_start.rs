use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::daemon::AppState;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
use crate::settings::ProviderControlMode;
use ctx_core::models::Session;
use ctx_core::provider_ids::CODEX_PROVIDER_ID;
use ctx_core::provider_policy::{CTX_CRP_LAUNCH_POLICY_ENV, CTX_CRP_LAUNCH_POLICY_FULL};

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
