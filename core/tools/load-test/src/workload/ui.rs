use std::collections::HashMap;
use std::future::Future;
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::time::sleep;

use ctx_client::{
    Client, ClientTelemetryBatch, ClientTelemetryMetric, PerfMetricKind,
    WorkspaceActiveSnapshotParams,
};

use crate::metrics::{build_ui_summary, UiMetrics, UiSummary};

pub(crate) async fn run_ui_workload(
    client: std::sync::Arc<Client>,
    workspace_id: ctx_core::ids::WorkspaceId,
    duration: Duration,
    interval: Duration,
    run_id: String,
) -> Result<UiSummary> {
    let metrics = UiMetrics::default();
    let mut telemetry = Vec::new();
    let deadline = Instant::now() + duration;
    let params = WorkspaceActiveSnapshotParams { limit: Some(50) };

    while Instant::now() < deadline {
        record_ui_call(
            &metrics,
            &mut telemetry,
            &run_id,
            "/api/workspaces",
            "GET",
            client.list_workspaces(),
        )
        .await;

        record_ui_call(
            &metrics,
            &mut telemetry,
            &run_id,
            "/api/workspaces/:id/active_snapshot",
            "GET",
            client.get_workspace_active_snapshot(workspace_id, &params),
        )
        .await;

        if telemetry.len() >= 100 {
            flush_client_telemetry(&client, &mut telemetry).await;
        }

        sleep(interval).await;
    }

    flush_client_telemetry(&client, &mut telemetry).await;

    Ok(build_ui_summary(&metrics))
}

async fn record_ui_call<F, T>(
    metrics: &UiMetrics,
    telemetry: &mut Vec<ClientTelemetryMetric>,
    run_id: &str,
    endpoint: &str,
    method: &str,
    future: F,
) where
    F: Future<Output = Result<T>>,
{
    let start = Instant::now();
    let result = future.await;
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    metrics.api_ms.lock().unwrap().push(elapsed);
    *metrics.sent.lock().unwrap() += 1;

    let (status, success) = if result.is_ok() {
        ("200".to_string(), true)
    } else {
        *metrics.errors.lock().unwrap() += 1;
        ("error".to_string(), false)
    };

    telemetry.push(ClientTelemetryMetric {
        name: "client.api.duration_ms".to_string(),
        kind: PerfMetricKind::Histogram,
        unit: "ms".to_string(),
        value: elapsed,
        labels: Some(client_labels(endpoint, method, &status, success)),
        run_id: Some(run_id.to_string()),
    });

    if !success {
        telemetry.push(ClientTelemetryMetric {
            name: "client.api.error_count".to_string(),
            kind: PerfMetricKind::Counter,
            unit: "count".to_string(),
            value: 1.0,
            labels: Some(client_labels(endpoint, method, "error", false)),
            run_id: Some(run_id.to_string()),
        });
    }
}

async fn flush_client_telemetry(client: &Client, telemetry: &mut Vec<ClientTelemetryMetric>) {
    if telemetry.is_empty() {
        return;
    }
    let batch = ClientTelemetryBatch {
        events: std::mem::take(telemetry),
    };
    let _ = client.post_client_telemetry(&batch).await;
}

fn client_labels(
    endpoint: &str,
    method: &str,
    status: &str,
    success: bool,
) -> HashMap<String, String> {
    let mut labels = HashMap::new();
    labels.insert("endpoint".to_string(), endpoint.to_string());
    labels.insert("method".to_string(), method.to_string());
    labels.insert("status".to_string(), status.to_string());
    labels.insert("success".to_string(), success.to_string());
    labels.insert("source".to_string(), "client".to_string());
    labels
}
