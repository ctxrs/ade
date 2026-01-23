use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use serde::Serialize;

use crate::scenario::{ScenarioMode, ScenarioSpec};

pub(crate) const HOT_READ_P95_TARGET_MS: f64 = 20.0;

#[derive(Default)]
pub(crate) struct Metrics {
    pub(crate) http_ms: Mutex<Vec<f64>>,
    pub(crate) first_chunk_ms: Mutex<Vec<f64>>,
    pub(crate) done_ms: Mutex<Vec<f64>>,
    pub(crate) sent: Mutex<u64>,
    pub(crate) done: Mutex<u64>,
    pub(crate) errors: Mutex<u64>,
}

#[derive(Default)]
pub(crate) struct ControlPlaneEndpointMetrics {
    pub(crate) api_ms: Mutex<Vec<f64>>,
    pub(crate) sent: Mutex<u64>,
    pub(crate) errors: Mutex<u64>,
}

#[derive(Default)]
pub(crate) struct ControlPlaneMetrics {
    pub(crate) totals: ControlPlaneEndpointMetrics,
    pub(crate) health: ControlPlaneEndpointMetrics,
    pub(crate) active_snapshot: ControlPlaneEndpointMetrics,
    pub(crate) session_snapshot: ControlPlaneEndpointMetrics,
    pub(crate) session_head: ControlPlaneEndpointMetrics,
    pub(crate) ws_replay: ControlPlaneEndpointMetrics,
    pub(crate) reconnect: ControlPlaneEndpointMetrics,
}

#[derive(Clone, Copy)]
pub(crate) enum ControlPlaneEndpoint {
    Health,
    ActiveSnapshot,
    SessionSnapshot,
    SessionHead,
    WsReplay,
    ReconnectCatchup,
}

impl ControlPlaneMetrics {
    pub(crate) fn endpoint_metrics(
        &self,
        endpoint: ControlPlaneEndpoint,
    ) -> &ControlPlaneEndpointMetrics {
        match endpoint {
            ControlPlaneEndpoint::Health => &self.health,
            ControlPlaneEndpoint::ActiveSnapshot => &self.active_snapshot,
            ControlPlaneEndpoint::SessionSnapshot => &self.session_snapshot,
            ControlPlaneEndpoint::SessionHead => &self.session_head,
            ControlPlaneEndpoint::WsReplay => &self.ws_replay,
            ControlPlaneEndpoint::ReconnectCatchup => &self.reconnect,
        }
    }
}

#[derive(Default)]
pub(crate) struct UiMetrics {
    pub(crate) api_ms: Mutex<Vec<f64>>,
    pub(crate) sent: Mutex<u64>,
    pub(crate) errors: Mutex<u64>,
}

#[derive(Default)]
pub(crate) struct PendingState {
    pub(crate) pending: HashMap<String, std::time::Instant>,
    pub(crate) first_chunked: HashSet<String>,
}

#[derive(Serialize)]
pub(crate) struct Summary {
    pub(crate) name: String,
    pub(crate) mode: ScenarioMode,
    pub(crate) duration_ms: u64,
    pub(crate) sent: u64,
    pub(crate) done: u64,
    pub(crate) errors: u64,
    pub(crate) http_ms: Percentiles,
    pub(crate) first_chunk_ms: Percentiles,
    pub(crate) done_ms: Percentiles,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) golden: Option<GoldenSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) control_plane: Option<ControlPlaneSummary>,
}

#[derive(Serialize)]
pub(crate) struct ControlPlaneSummary {
    pub(crate) sent: u64,
    pub(crate) errors: u64,
    pub(crate) api_ms: Percentiles,
    pub(crate) endpoints: ControlPlaneEndpointSummaryBreakdown,
}

#[derive(Serialize)]
pub(crate) struct GoldenSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) hot_read_p95_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) hot_read_p95_target_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) hot_read_p95_ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reconnect_catchup_p95_ms: Option<f64>,
}

#[derive(Serialize)]
pub(crate) struct UiSummary {
    pub(crate) sent: u64,
    pub(crate) errors: u64,
    pub(crate) api_ms: Percentiles,
}

#[derive(Serialize)]
pub(crate) struct ControlPlaneEndpointSummary {
    pub(crate) sent: u64,
    pub(crate) errors: u64,
    pub(crate) api_ms: Percentiles,
}

#[derive(Serialize)]
pub(crate) struct ControlPlaneEndpointSummaryBreakdown {
    pub(crate) health: ControlPlaneEndpointSummary,
    pub(crate) active_snapshot: ControlPlaneEndpointSummary,
    pub(crate) session_snapshot: ControlPlaneEndpointSummary,
    pub(crate) session_head: ControlPlaneEndpointSummary,
    pub(crate) ws_replay: ControlPlaneEndpointSummary,
    pub(crate) reconnect: ControlPlaneEndpointSummary,
}

#[derive(Serialize)]
pub(crate) struct CombinedSummary {
    pub(crate) name: String,
    pub(crate) mode: ScenarioMode,
    pub(crate) duration_ms: u64,
    pub(crate) run_id: Option<String>,
    pub(crate) daemon: Summary,
    pub(crate) ui: Option<UiSummary>,
    pub(crate) ui_telemetry: Option<serde_json::Value>,
}

#[derive(Serialize, Default)]
pub(crate) struct Percentiles {
    pub(crate) p50: f64,
    pub(crate) p95: f64,
    pub(crate) p99: f64,
    pub(crate) max: f64,
}

pub(crate) fn build_ui_summary(metrics: &UiMetrics) -> UiSummary {
    let sent = *metrics.sent.lock().unwrap();
    let errors = *metrics.errors.lock().unwrap();
    UiSummary {
        sent,
        errors,
        api_ms: summarize(metrics.api_ms.lock().unwrap().as_slice()),
    }
}

pub(crate) fn build_control_plane_summary(metrics: &ControlPlaneMetrics) -> ControlPlaneSummary {
    let totals = summarize_control_plane_endpoint(&metrics.totals);
    ControlPlaneSummary {
        sent: totals.sent,
        errors: totals.errors,
        api_ms: totals.api_ms,
        endpoints: ControlPlaneEndpointSummaryBreakdown {
            health: summarize_control_plane_endpoint(&metrics.health),
            active_snapshot: summarize_control_plane_endpoint(&metrics.active_snapshot),
            session_snapshot: summarize_control_plane_endpoint(&metrics.session_snapshot),
            session_head: summarize_control_plane_endpoint(&metrics.session_head),
            ws_replay: summarize_control_plane_endpoint(&metrics.ws_replay),
            reconnect: summarize_control_plane_endpoint(&metrics.reconnect),
        },
    }
}

fn summarize_control_plane_endpoint(
    metrics: &ControlPlaneEndpointMetrics,
) -> ControlPlaneEndpointSummary {
    ControlPlaneEndpointSummary {
        sent: *metrics.sent.lock().unwrap(),
        errors: *metrics.errors.lock().unwrap(),
        api_ms: summarize(metrics.api_ms.lock().unwrap().as_slice()),
    }
}

pub(crate) fn build_summary(
    scenario: &ScenarioSpec,
    metrics: &Metrics,
    control_plane: Option<ControlPlaneSummary>,
) -> Summary {
    let sent = *metrics.sent.lock().unwrap();
    let done = *metrics.done.lock().unwrap();
    let errors = *metrics.errors.lock().unwrap();
    let golden = build_golden_summary(control_plane.as_ref());
    Summary {
        name: scenario.name.clone(),
        mode: scenario.mode,
        duration_ms: scenario.duration_ms,
        sent,
        done,
        errors,
        http_ms: summarize(metrics.http_ms.lock().unwrap().as_slice()),
        first_chunk_ms: summarize(metrics.first_chunk_ms.lock().unwrap().as_slice()),
        done_ms: summarize(metrics.done_ms.lock().unwrap().as_slice()),
        golden,
        control_plane,
    }
}

fn build_golden_summary(control_plane: Option<&ControlPlaneSummary>) -> Option<GoldenSummary> {
    let control_plane = control_plane?;
    let active_snapshot = &control_plane.endpoints.active_snapshot;
    let session_head = &control_plane.endpoints.session_head;
    let hot_read_p95 = if active_snapshot.sent > 0 && session_head.sent > 0 {
        Some(active_snapshot.api_ms.p95.max(session_head.api_ms.p95))
    } else {
        None
    };
    let reconnect_p95 = if control_plane.endpoints.reconnect.sent > 0 {
        Some(control_plane.endpoints.reconnect.api_ms.p95)
    } else {
        None
    };

    if hot_read_p95.is_none() && reconnect_p95.is_none() {
        return None;
    }

    Some(GoldenSummary {
        hot_read_p95_ms: hot_read_p95,
        hot_read_p95_target_ms: hot_read_p95.map(|_| HOT_READ_P95_TARGET_MS),
        hot_read_p95_ok: hot_read_p95.map(|value| value <= HOT_READ_P95_TARGET_MS),
        reconnect_catchup_p95_ms: reconnect_p95,
    })
}

fn summarize(values: &[f64]) -> Percentiles {
    if values.is_empty() {
        return Percentiles::default();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Percentiles {
        p50: percentile(&sorted, 0.50),
        p95: percentile(&sorted, 0.95),
        p99: percentile(&sorted, 0.99),
        max: *sorted.last().unwrap_or(&0.0),
    }
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((sorted.len() - 1) as f64 * pct).round() as usize;
    sorted.get(rank).cloned().unwrap_or(0.0)
}
