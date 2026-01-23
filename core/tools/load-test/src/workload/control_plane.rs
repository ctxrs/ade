use std::future::Future;
use std::time::Instant;

use anyhow::Result;

use ctx_client::{Client, WorkspaceActiveSnapshotParams};
use ctx_core::ids::WorkspaceId;

use crate::metrics::{ControlPlaneEndpoint, ControlPlaneEndpointMetrics, ControlPlaneMetrics};
use crate::output::{EventRecord, EventWriter};

pub(crate) const ACTIVE_SNAPSHOT_LIMIT: u32 = 50;
pub(crate) const SESSION_SNAPSHOT_LIMIT: u32 = 40;
pub(crate) const SESSION_HEAD_LIMIT: u32 = 40;

pub(crate) fn record_control_plane_metrics(
    metrics: &ControlPlaneEndpointMetrics,
    elapsed: f64,
    success: bool,
) {
    metrics.api_ms.lock().unwrap().push(elapsed);
    *metrics.sent.lock().unwrap() += 1;
    if !success {
        *metrics.errors.lock().unwrap() += 1;
    }
}

pub(crate) fn record_control_plane_result(
    metrics: &ControlPlaneMetrics,
    endpoint: ControlPlaneEndpoint,
    events: &mut EventWriter,
    kind: &str,
    elapsed: f64,
    success: bool,
) {
    record_control_plane_metrics(&metrics.totals, elapsed, success);
    record_control_plane_metrics(metrics.endpoint_metrics(endpoint), elapsed, success);
    events
        .write(EventRecord {
            ts_ms: chrono::Utc::now().timestamp_millis() as u128,
            kind,
            value_ms: Some(elapsed),
            detail: if success {
                None
            } else {
                Some("error".to_string())
            },
        })
        .ok();
}

pub(crate) async fn record_control_call<F, T>(
    metrics: &ControlPlaneMetrics,
    events: &mut EventWriter,
    endpoint: ControlPlaneEndpoint,
    kind: &str,
    future: F,
) where
    F: Future<Output = Result<T>>,
{
    let start = Instant::now();
    let result = future.await;
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    record_control_plane_result(metrics, endpoint, events, kind, elapsed, result.is_ok());
}

pub(crate) async fn run_reconnect_catchup_once(
    client: &Client,
    workspace_id: WorkspaceId,
    params: &WorkspaceActiveSnapshotParams,
) -> Result<()> {
    let _ = client
        .get_workspace_active_snapshot(workspace_id, params)
        .await?;
    let _ = client.get_workspace_active_heads(workspace_id).await?;
    Ok(())
}
