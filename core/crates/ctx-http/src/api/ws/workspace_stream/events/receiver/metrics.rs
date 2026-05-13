use super::*;
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind};
use std::collections::HashMap;

pub(super) async fn record_workspace_stream_receiver_drain(
    state: &Arc<AppState>,
    labels: &WorkspaceStreamLabels,
    event_count: usize,
    hit_limit: bool,
) {
    if event_count <= 1 && !hit_limit {
        return;
    }
    let mut metric_labels = HashMap::new();
    metric_labels.insert("source".to_string(), "daemon".to_string());
    metric_labels.insert(
        "queue_label".to_string(),
        labels.event_queue_label.to_string(),
    );
    metric_labels.insert(
        "hit_limit".to_string(),
        if hit_limit { "true" } else { "false" }.to_string(),
    );
    state
        .telemetry
        .perf_telemetry
        .record_metric(
            PerfMetric {
                name: "workspace.stream.receiver_drain_event_count".to_string(),
                kind: PerfMetricKind::Histogram,
                unit: "count".to_string(),
                value: event_count as f64,
                labels: metric_labels,
            },
            None,
            None,
            None,
        )
        .await;
}
