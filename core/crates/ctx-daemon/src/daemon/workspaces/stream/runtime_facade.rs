use std::collections::HashMap;

use ctx_core::ids::SessionId;
use ctx_observability::telemetry::TelemetryEvent;

use crate::daemon::WorkspaceStreamHandle;

impl WorkspaceStreamHandle {
    pub async fn attach_session_pin(&self, session_id: SessionId) {
        self.state.attach_session(session_id).await;
    }

    pub async fn detach_session_pin(&self, session_id: SessionId) {
        self.state.detach_session(session_id).await;
    }

    pub async fn apply_workspace_stream_session_pin_changes(
        &self,
        pin_changes: &super::WorkspaceStreamSessionPinChanges,
    ) {
        for session_id in &pin_changes.attach {
            self.state.attach_session(*session_id).await;
        }
        for session_id in &pin_changes.detach {
            self.state.detach_session(*session_id).await;
        }
    }

    pub async fn release_workspace_stream_session_pins<I>(&self, session_ids: I)
    where
        I: IntoIterator<Item = SessionId>,
    {
        for session_id in session_ids {
            self.state.detach_session(session_id).await;
        }
    }

    pub async fn emit_workspace_stream_incident(
        &self,
        event_name: &'static str,
        labels: &[(&'static str, serde_json::Value)],
    ) {
        let mut event = TelemetryEvent::daemon_incident(event_name)
            .with_source("workspace_stream")
            .with_property("has_workspace_scope", serde_json::json!(true));
        for (key, value) in labels {
            event = event.with_property(*key, value.clone());
        }
        self.state.telemetry.telemetry.emit(event).await;
    }

    pub async fn record_workspace_stream_receiver_drain(
        &self,
        queue_label: &'static str,
        event_count: usize,
        hit_limit: bool,
    ) {
        let mut labels = HashMap::new();
        labels.insert("source".to_string(), "daemon".to_string());
        labels.insert("queue_label".to_string(), queue_label.to_string());
        labels.insert(
            "hit_limit".to_string(),
            if hit_limit { "true" } else { "false" }.to_string(),
        );
        self.state
            .telemetry
            .perf_telemetry
            .record_metric(
                ctx_observability::perf_telemetry::PerfMetric {
                    name: "workspace.stream.receiver_drain_event_count".to_string(),
                    kind: ctx_observability::perf_telemetry::PerfMetricKind::Histogram,
                    unit: "count".to_string(),
                    value: event_count as f64,
                    labels,
                },
                None,
                None,
                None,
            )
            .await;
    }
}
