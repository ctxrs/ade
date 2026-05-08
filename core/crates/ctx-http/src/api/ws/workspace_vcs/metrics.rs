use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;

use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind};

use super::buffer::vcs_stream_message_is_snapshot;
use super::*;

#[derive(Default)]
pub(super) struct VcsStreamMetrics {
    pub(super) snapshot_queued_count: AtomicU64,
    pub(super) snapshot_coalesced_count: AtomicU64,
    message_sent_count: AtomicU64,
    snapshot_sent_count: AtomicU64,
}

impl VcsStreamMetrics {
    pub(super) fn snapshot_queued(&self, coalesced: bool) {
        self.snapshot_queued_count
            .fetch_add(1, AtomicOrdering::Relaxed);
        if coalesced {
            self.snapshot_coalesced_count
                .fetch_add(1, AtomicOrdering::Relaxed);
        }
    }

    pub(super) fn message_sent(&self, message: &WorktreeVcsStreamMessage) {
        self.message_sent_count
            .fetch_add(1, AtomicOrdering::Relaxed);
        if vcs_stream_message_is_snapshot(message) {
            self.snapshot_sent_count
                .fetch_add(1, AtomicOrdering::Relaxed);
        }
    }
}

pub(super) async fn record_workspace_vcs_stream_metrics(
    state: &Arc<AppState>,
    metrics: &VcsStreamMetrics,
) {
    let counters = [
        (
            "workspace.vcs_stream.server_snapshot_queued_count",
            metrics.snapshot_queued_count.load(AtomicOrdering::Relaxed),
        ),
        (
            "workspace.vcs_stream.server_snapshot_coalesced_count",
            metrics
                .snapshot_coalesced_count
                .load(AtomicOrdering::Relaxed),
        ),
        (
            "workspace.vcs_stream.server_message_sent_count",
            metrics.message_sent_count.load(AtomicOrdering::Relaxed),
        ),
        (
            "workspace.vcs_stream.server_snapshot_sent_count",
            metrics.snapshot_sent_count.load(AtomicOrdering::Relaxed),
        ),
    ];
    for (name, value) in counters {
        if value == 0 {
            continue;
        }
        let mut labels = HashMap::new();
        labels.insert("source".to_string(), "daemon".to_string());
        labels.insert("stream".to_string(), "workspace_vcs".to_string());
        let metric = PerfMetric {
            name: name.to_string(),
            kind: PerfMetricKind::Counter,
            unit: "count".to_string(),
            value: value as f64,
            labels,
        };
        state
            .telemetry
            .perf_telemetry
            .record_metric(metric, None, None, None)
            .await;
    }
}
