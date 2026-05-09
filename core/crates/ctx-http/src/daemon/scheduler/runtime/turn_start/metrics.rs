use std::collections::HashMap;
use std::sync::Arc;

use ctx_core::models::Session;
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind};

use crate::daemon::AppState;

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
