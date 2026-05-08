use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use ctx_core::models::SessionHeadSnapshot;
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind};

use crate::daemon::AppState;

pub(super) fn record_session_head_recovery_metrics(
    state: &Arc<AppState>,
    source: &'static str,
    result: &'static str,
    elapsed: Duration,
    limit: u32,
    include_events: bool,
    head: Option<&SessionHeadSnapshot>,
) {
    let mut labels = HashMap::new();
    labels.insert("source".to_string(), "daemon".to_string());
    labels.insert("surface".to_string(), "session_head_recovery".to_string());
    labels.insert("recovery_source".to_string(), source.to_string());
    labels.insert("result".to_string(), result.to_string());
    labels.insert(
        "include_events".to_string(),
        if include_events { "true" } else { "false" }.to_string(),
    );
    labels.insert(
        "limit_bucket".to_string(),
        session_head_limit_bucket(limit).to_string(),
    );

    let response_bytes = head
        .map(|value| value.head_window.bytes.max(0) as f64)
        .unwrap_or(0.0);
    let metrics = [
        (
            "workbench.session_head_recovery_ms",
            "ms",
            elapsed.as_millis() as f64,
        ),
        (
            "workbench.session_head_recovery_response_bytes",
            "bytes",
            response_bytes,
        ),
        (
            "workbench.session_head_recovery_turn_count",
            "count",
            head.map(|value| value.turns.len() as f64).unwrap_or(0.0),
        ),
        (
            "workbench.session_head_recovery_message_count",
            "count",
            head.map(|value| value.messages.len() as f64).unwrap_or(0.0),
        ),
        (
            "workbench.session_head_recovery_tool_summary_count",
            "count",
            head.map(|value| value.tool_summaries.len() as f64)
                .unwrap_or(0.0),
        ),
        (
            "workbench.session_head_recovery_event_count",
            "count",
            head.map(|value| value.events.len() as f64).unwrap_or(0.0),
        ),
    ];
    let perf_telemetry = state.telemetry.perf_telemetry.clone();
    tokio::spawn(async move {
        for (name, unit, value) in metrics {
            perf_telemetry
                .record_metric(
                    PerfMetric {
                        name: name.to_string(),
                        kind: PerfMetricKind::Histogram,
                        unit: unit.to_string(),
                        value,
                        labels: labels.clone(),
                    },
                    None,
                    None,
                    None,
                )
                .await;
        }
    });
}

fn session_head_limit_bucket(limit: u32) -> &'static str {
    match limit {
        0 => "zero",
        1..=5 => "1_5",
        6..=60 => "6_60",
        61..=200 => "61_200",
        _ => "gt_200",
    }
}
