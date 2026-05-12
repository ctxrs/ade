use super::*;
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind};
use std::collections::HashMap;

const WORKSPACE_STREAM_RECEIVER_DRAIN_LIMIT: usize = 2048;

#[derive(Debug)]
pub(crate) struct WorkspaceStreamReceiverBurst {
    events: Vec<WorkspaceActiveSnapshotEvent>,
    lagged: Option<u64>,
    closed: bool,
    hit_limit: bool,
}

pub(crate) fn take_workspace_stream_receiver_burst(
    rx: &mut tokio::sync::broadcast::Receiver<WorkspaceActiveSnapshotEvent>,
    first_event: WorkspaceActiveSnapshotEvent,
) -> WorkspaceStreamReceiverBurst {
    let mut events = vec![first_event];
    let mut hit_limit = false;
    for _ in 1..WORKSPACE_STREAM_RECEIVER_DRAIN_LIMIT {
        match rx.try_recv() {
            Ok(event) => events.push(event),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                return WorkspaceStreamReceiverBurst {
                    events,
                    lagged: None,
                    closed: false,
                    hit_limit,
                };
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(lagged)) => {
                return WorkspaceStreamReceiverBurst {
                    events,
                    lagged: Some(lagged),
                    closed: false,
                    hit_limit,
                };
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                return WorkspaceStreamReceiverBurst {
                    events,
                    lagged: None,
                    closed: true,
                    hit_limit,
                };
            }
        }
    }
    if !events.is_empty() {
        hit_limit = true;
    }
    WorkspaceStreamReceiverBurst {
        events,
        lagged: None,
        closed: false,
        hit_limit,
    }
}

pub(crate) async fn handle_workspace_stream_receiver_burst(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    burst: WorkspaceStreamReceiverBurst,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
) -> Result<(), ()> {
    let event_count = burst.events.len();
    let hit_limit = burst.hit_limit;
    for event in burst.events {
        handle_workspace_stream_event(state, workspace_id, event, runtime, labels).await?;
        if runtime.reset_queued || runtime.send_control.should_disconnect_after_flush() {
            record_workspace_stream_receiver_drain(state, labels, event_count, hit_limit).await;
            return Ok(());
        }
    }
    record_workspace_stream_receiver_drain(state, labels, event_count, hit_limit).await;
    if let Some(lagged) = burst.lagged {
        handle_workspace_stream_lagged(state, workspace_id, lagged, runtime, labels).await?;
    }
    if burst.closed {
        return Err(());
    }
    Ok(())
}

pub(crate) async fn drain_pending_workspace_stream_receiver_burst_deferring<F>(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    rx: &mut tokio::sync::broadcast::Receiver<WorkspaceActiveSnapshotEvent>,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
    deferred_events: &mut Vec<WorkspaceActiveSnapshotEvent>,
    mut should_defer: F,
) -> Result<(), ()>
where
    F: FnMut(&WorkspaceActiveSnapshotEvent) -> bool,
{
    match rx.try_recv() {
        Ok(event) => {
            let burst = take_workspace_stream_receiver_burst(rx, event);
            handle_workspace_stream_receiver_burst_deferring(
                state,
                workspace_id,
                burst,
                runtime,
                labels,
                deferred_events,
                &mut should_defer,
            )
            .await
        }
        Err(tokio::sync::broadcast::error::TryRecvError::Empty) => Ok(()),
        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(lagged)) => {
            handle_workspace_stream_lagged(state, workspace_id, lagged, runtime, labels).await
        }
        Err(tokio::sync::broadcast::error::TryRecvError::Closed) => Err(()),
    }
}

pub(crate) async fn flush_deferred_workspace_stream_receiver_events<F>(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
    deferred_events: &mut Vec<WorkspaceActiveSnapshotEvent>,
    mut should_defer: F,
) -> Result<(), ()>
where
    F: FnMut(&WorkspaceActiveSnapshotEvent) -> bool,
{
    let ready_count = deferred_events
        .iter()
        .take_while(|event| !should_defer(event))
        .count();
    if ready_count == 0 {
        return Ok(());
    }
    let events = deferred_events.drain(..ready_count).collect::<Vec<_>>();
    handle_workspace_stream_receiver_burst(
        state,
        workspace_id,
        WorkspaceStreamReceiverBurst {
            events,
            lagged: None,
            closed: false,
            hit_limit: false,
        },
        runtime,
        labels,
    )
    .await
}

async fn handle_workspace_stream_receiver_burst_deferring<F>(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    burst: WorkspaceStreamReceiverBurst,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
    deferred_events: &mut Vec<WorkspaceActiveSnapshotEvent>,
    should_defer: &mut F,
) -> Result<(), ()>
where
    F: FnMut(&WorkspaceActiveSnapshotEvent) -> bool,
{
    let event_count = burst.events.len();
    let hit_limit = burst.hit_limit;
    let mut ready_events = Vec::new();
    let mut defer_remaining = false;
    for event in burst.events {
        if defer_remaining || should_defer(&event) {
            defer_remaining = true;
            deferred_events.push(event);
        } else {
            ready_events.push(event);
        }
    }
    if !ready_events.is_empty() {
        handle_workspace_stream_receiver_burst(
            state,
            workspace_id,
            WorkspaceStreamReceiverBurst {
                events: ready_events,
                lagged: None,
                closed: false,
                hit_limit,
            },
            runtime,
            labels,
        )
        .await?;
    } else {
        record_workspace_stream_receiver_drain(state, labels, event_count, hit_limit).await;
    }
    if let Some(lagged) = burst.lagged {
        handle_workspace_stream_lagged(state, workspace_id, lagged, runtime, labels).await?;
    }
    if burst.closed {
        return Err(());
    }
    Ok(())
}

async fn record_workspace_stream_receiver_drain(
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_event(workspace_id: WorkspaceId, snapshot_rev: i64) -> WorkspaceActiveSnapshotEvent {
        WorkspaceActiveSnapshotEvent::Ready {
            workspace_id,
            snapshot_rev,
            archived_rev: 0,
        }
    }

    #[test]
    fn receiver_burst_drains_ready_events_up_to_fairness_limit() {
        let workspace_id = WorkspaceId::new();
        let (tx, mut rx) =
            tokio::sync::broadcast::channel(WORKSPACE_STREAM_RECEIVER_DRAIN_LIMIT + 16);
        for rev in 1..=(WORKSPACE_STREAM_RECEIVER_DRAIN_LIMIT + 4) {
            tx.send(ready_event(
                workspace_id,
                i64::try_from(rev).expect("rev fits in i64"),
            ))
            .expect("receiver is open");
        }

        let first_event = rx.try_recv().expect("first event is ready");
        let burst = take_workspace_stream_receiver_burst(&mut rx, first_event);

        assert_eq!(burst.events.len(), WORKSPACE_STREAM_RECEIVER_DRAIN_LIMIT);
        assert!(burst.hit_limit);
        assert_eq!(burst.lagged, None);
        assert!(!burst.closed);
        assert!(matches!(
            rx.try_recv(),
            Ok(WorkspaceActiveSnapshotEvent::Ready { .. })
        ));
    }

    #[test]
    fn receiver_burst_reports_lag_after_collected_events() {
        let workspace_id = WorkspaceId::new();
        let (tx, mut rx) = tokio::sync::broadcast::channel(4);
        tx.send(ready_event(workspace_id, 1))
            .expect("receiver is open");
        let first_event = rx.try_recv().expect("first event is ready");
        for rev in 2..=8 {
            tx.send(ready_event(workspace_id, rev))
                .expect("receiver is open");
        }

        let burst = take_workspace_stream_receiver_burst(&mut rx, first_event);

        assert_eq!(burst.events.len(), 1);
        assert_eq!(burst.lagged, Some(3));
        assert!(!burst.closed);
        assert!(!burst.hit_limit);
    }
}
