use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use super::*;
use crate::daemon::workspaces::stream::refresh_worktree_vcs_for_worktrees;
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct VcsSnapshotKey {
    worktree_id: WorktreeId,
    tier: WorktreeVcsStreamTier,
}

#[derive(Default)]
struct VcsPendingState {
    controls: VecDeque<WorktreeVcsStreamMessage>,
    snapshots: HashMap<VcsSnapshotKey, WorktreeVcsStreamMessage>,
}

struct VcsPendingBuffer {
    state: Mutex<VcsPendingState>,
    notify: Notify,
}

#[derive(Default)]
struct VcsStreamMetrics {
    snapshot_queued_count: AtomicU64,
    snapshot_coalesced_count: AtomicU64,
    message_sent_count: AtomicU64,
    snapshot_sent_count: AtomicU64,
}

impl VcsStreamMetrics {
    fn snapshot_queued(&self, coalesced: bool) {
        self.snapshot_queued_count
            .fetch_add(1, AtomicOrdering::Relaxed);
        if coalesced {
            self.snapshot_coalesced_count
                .fetch_add(1, AtomicOrdering::Relaxed);
        }
    }

    fn message_sent(&self, message: &WorktreeVcsStreamMessage) {
        self.message_sent_count
            .fetch_add(1, AtomicOrdering::Relaxed);
        if vcs_stream_message_is_snapshot(message) {
            self.snapshot_sent_count
                .fetch_add(1, AtomicOrdering::Relaxed);
        }
    }
}

impl VcsPendingBuffer {
    fn new() -> Self {
        Self {
            state: Mutex::new(VcsPendingState::default()),
            notify: Notify::new(),
        }
    }

    async fn push_control(&self, message: WorktreeVcsStreamMessage) {
        let mut state = self.state.lock().await;
        state.controls.push_back(message);
        self.notify.notify_one();
    }

    async fn push_snapshot(&self, key: VcsSnapshotKey, message: WorktreeVcsStreamMessage) -> bool {
        let mut state = self.state.lock().await;
        let coalesced = state.snapshots.insert(key, message).is_some();
        self.notify.notify_one();
        coalesced
    }

    async fn pop(&self) -> Option<WorktreeVcsStreamMessage> {
        let mut state = self.state.lock().await;
        if let Some(message) = state.controls.pop_front() {
            return Some(message);
        }
        let key = state.snapshots.keys().next().copied()?;
        state.snapshots.remove(&key)
    }

    #[cfg(test)]
    async fn is_empty(&self) -> bool {
        let state = self.state.lock().await;
        state.controls.is_empty() && state.snapshots.is_empty()
    }
}

fn vcs_stream_message_is_snapshot(message: &WorktreeVcsStreamMessage) -> bool {
    matches!(
        message,
        WorktreeVcsStreamMessage::SummarySnapshot { .. }
            | WorktreeVcsStreamMessage::DetailsSnapshot { .. }
            | WorktreeVcsStreamMessage::UnavailableSnapshot { .. }
    )
}

async fn record_workspace_vcs_stream_metrics(state: &Arc<AppState>, metrics: &VcsStreamMetrics) {
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

#[derive(Default)]
struct WorkspaceVcsRuntime {
    demand_generation: i64,
    summary_worktree_ids: HashSet<WorktreeId>,
    detail_worktree_ids: HashSet<WorktreeId>,
}

impl WorkspaceVcsRuntime {
    fn active_worktree_ids(&self) -> HashSet<WorktreeId> {
        self.summary_worktree_ids
            .union(&self.detail_worktree_ids)
            .copied()
            .collect()
    }
}

async fn require_workspace_vcs_stream_access(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> Result<(), StatusCode> {
    let exists = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some();
    if !exists {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(())
}

pub(crate) async fn workspace_vcs_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workspace_id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => WorkspaceId(value),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if let Err(status) = require_workspace_vcs_stream_access(&state, workspace_id).await {
        return status.into_response();
    }
    ws.on_upgrade(move |socket| handle_workspace_vcs_ws(socket, state, workspace_id))
}

async fn handle_workspace_vcs_ws(
    socket: WebSocket,
    state: Arc<AppState>,
    workspace_id: WorkspaceId,
) {
    let (sender, mut receiver) = socket.split();
    let pending = Arc::new(VcsPendingBuffer::new());
    let metrics = Arc::new(VcsStreamMetrics::default());
    pending
        .push_control(WorktreeVcsStreamMessage::Ready {
            workspace_id,
            vcs_generation: 0,
        })
        .await;

    let send_task = {
        let pending = Arc::clone(&pending);
        let metrics = Arc::clone(&metrics);
        tokio::spawn(async move {
            let mut sender = sender;
            loop {
                if let Some(message) = pending.pop().await {
                    let Ok(text) = serde_json::to_string(&message) else {
                        break;
                    };
                    if sender.send(WsMessage::Text(text)).await.is_err() {
                        break;
                    }
                    metrics.message_sent(&message);
                    continue;
                }
                pending.notify.notified().await;
            }
        })
    };

    let mut runtime = WorkspaceVcsRuntime::default();
    let mut rx = state.workspaces.worktree_vcs_events.subscribe();
    let recv_loop = async {
        loop {
            tokio::select! {
                msg = receiver.next() => {
                    match msg {
                        Some(Ok(WsMessage::Text(text))) => {
                            let Ok(message) = serde_json::from_str::<WorktreeVcsStreamClientMessage>(&text) else {
                                continue;
                            };
                            handle_workspace_vcs_client_message(
                                &state,
                                workspace_id,
                                &pending,
                                &metrics,
                                &mut runtime,
                                message,
                            )
                            .await;
                        }
                        Some(Ok(WsMessage::Binary(bytes))) => {
                            let Ok(text) = String::from_utf8(bytes.to_vec()) else {
                                continue;
                            };
                            let Ok(message) = serde_json::from_str::<WorktreeVcsStreamClientMessage>(&text) else {
                                continue;
                            };
                            handle_workspace_vcs_client_message(
                                &state,
                                workspace_id,
                                &pending,
                                &metrics,
                                &mut runtime,
                                message,
                            )
                            .await;
                        }
                        Some(Ok(WsMessage::Close(_))) => break,
                        Some(Ok(_)) => {}
                        Some(Err(_)) | None => break,
                    }
                }
                event = rx.recv() => {
                    match event {
                        Ok(snapshot) => {
                            if runtime.detail_worktree_ids.contains(&snapshot.worktree_id) {
                                queue_vcs_snapshot(
                                    &pending,
                                    &metrics,
                                    workspace_id,
                                    runtime.demand_generation,
                                    WorktreeVcsStreamTier::Details,
                                    snapshot,
                                )
                                .await;
                            } else if runtime.summary_worktree_ids.contains(&snapshot.worktree_id) {
                                queue_vcs_snapshot(
                                    &pending,
                                    &metrics,
                                    workspace_id,
                                    runtime.demand_generation,
                                    WorktreeVcsStreamTier::Summary,
                                    snapshot,
                                )
                                .await;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            tracing::warn!(
                                target: "ctx_http.ws_vcs",
                                workspace_id = %workspace_id.0,
                                skipped,
                                "workspace vcs stream lagged; latest subscribed snapshots will be reseeded",
                            );
                            seed_current_vcs_snapshots(
                                &state,
                                workspace_id,
                                &pending,
                                &metrics,
                                runtime.demand_generation,
                                &runtime.summary_worktree_ids,
                                &runtime.detail_worktree_ids,
                            )
                            .await;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
    };

    recv_loop.await;
    send_task.abort();
    record_workspace_vcs_stream_metrics(&state, &metrics).await;
    release_workspace_vcs_demand(&state, &runtime).await;
}

async fn handle_workspace_vcs_client_message(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    pending: &Arc<VcsPendingBuffer>,
    metrics: &Arc<VcsStreamMetrics>,
    runtime: &mut WorkspaceVcsRuntime,
    message: WorktreeVcsStreamClientMessage,
) {
    match message {
        WorktreeVcsStreamClientMessage::ReplaceSubscription {
            summary_worktree_ids,
            detail_worktree_ids,
        } => {
            replace_workspace_vcs_subscription(
                state,
                workspace_id,
                pending,
                metrics,
                runtime,
                summary_worktree_ids,
                detail_worktree_ids,
            )
            .await;
        }
        WorktreeVcsStreamClientMessage::Refresh { worktree_ids, tier } => {
            let worktree_ids =
                filter_workspace_worktree_ids(state, workspace_id, worktree_ids).await;
            match tier {
                WorktreeVcsStreamTier::Summary => {
                    refresh_worktree_vcs_for_worktrees(state, &worktree_ids, &[]).await;
                }
                WorktreeVcsStreamTier::Details => {
                    refresh_worktree_vcs_for_worktrees(state, &[], &worktree_ids).await;
                }
            }
        }
    }
}

async fn replace_workspace_vcs_subscription(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    pending: &Arc<VcsPendingBuffer>,
    metrics: &Arc<VcsStreamMetrics>,
    runtime: &mut WorkspaceVcsRuntime,
    summary_worktree_ids: Vec<WorktreeId>,
    detail_worktree_ids: Vec<WorktreeId>,
) {
    let previous_active = runtime.active_worktree_ids();
    let previous_details = runtime.detail_worktree_ids.clone();
    let summary_worktree_ids =
        filter_workspace_worktree_ids(state, workspace_id, summary_worktree_ids).await;
    let detail_worktree_ids =
        filter_workspace_worktree_ids(state, workspace_id, detail_worktree_ids).await;
    runtime.summary_worktree_ids = summary_worktree_ids.iter().copied().collect();
    runtime.detail_worktree_ids = detail_worktree_ids.iter().copied().collect();
    let next_active = runtime.active_worktree_ids();
    state
        .update_worktree_vcs_activity(&previous_active, &next_active)
        .await;
    state
        .update_worktree_vcs_open_panes(&previous_details, &runtime.detail_worktree_ids)
        .await;
    runtime.demand_generation += 1;

    pending
        .push_control(WorktreeVcsStreamMessage::Subscribed {
            workspace_id,
            demand_generation: runtime.demand_generation,
            summary_worktree_ids: summary_worktree_ids.clone(),
            detail_worktree_ids: detail_worktree_ids.clone(),
        })
        .await;
    seed_current_vcs_snapshots(
        state,
        workspace_id,
        pending,
        metrics,
        runtime.demand_generation,
        &runtime.summary_worktree_ids,
        &runtime.detail_worktree_ids,
    )
    .await;
    refresh_worktree_vcs_for_worktrees(state, &summary_worktree_ids, &detail_worktree_ids).await;
}

async fn release_workspace_vcs_demand(state: &Arc<AppState>, runtime: &WorkspaceVcsRuntime) {
    let active = runtime.active_worktree_ids();
    if active.is_empty() && runtime.detail_worktree_ids.is_empty() {
        return;
    }
    state
        .update_worktree_vcs_activity(&active, &HashSet::new())
        .await;
    state
        .update_worktree_vcs_open_panes(&runtime.detail_worktree_ids, &HashSet::new())
        .await;
}

async fn seed_current_vcs_snapshots(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    pending: &Arc<VcsPendingBuffer>,
    metrics: &Arc<VcsStreamMetrics>,
    demand_generation: i64,
    summary_worktree_ids: &HashSet<WorktreeId>,
    detail_worktree_ids: &HashSet<WorktreeId>,
) {
    let mut worktree_ids: Vec<_> = summary_worktree_ids
        .union(detail_worktree_ids)
        .copied()
        .collect();
    worktree_ids.sort_by_key(|worktree_id| worktree_id.0);
    for worktree_id in worktree_ids {
        let Some(snapshot) = state.get_worktree_vcs_snapshot(worktree_id).await else {
            continue;
        };
        let tier = if detail_worktree_ids.contains(&worktree_id) {
            WorktreeVcsStreamTier::Details
        } else {
            WorktreeVcsStreamTier::Summary
        };
        queue_vcs_snapshot(
            pending,
            metrics,
            workspace_id,
            demand_generation,
            tier,
            snapshot,
        )
        .await;
    }
}

async fn queue_vcs_snapshot(
    pending: &Arc<VcsPendingBuffer>,
    metrics: &Arc<VcsStreamMetrics>,
    workspace_id: WorkspaceId,
    demand_generation: i64,
    tier: WorktreeVcsStreamTier,
    snapshot: WorktreeVcsSnapshot,
) {
    let worktree_id = snapshot.worktree_id;
    let message = if snapshot.available {
        match tier {
            WorktreeVcsStreamTier::Summary => WorktreeVcsStreamMessage::SummarySnapshot {
                workspace_id,
                worktree_id,
                demand_generation,
                snapshot,
            },
            WorktreeVcsStreamTier::Details => WorktreeVcsStreamMessage::DetailsSnapshot {
                workspace_id,
                worktree_id,
                demand_generation,
                snapshot,
            },
        }
    } else {
        WorktreeVcsStreamMessage::UnavailableSnapshot {
            workspace_id,
            worktree_id,
            demand_generation,
            snapshot,
        }
    };
    let coalesced = pending
        .push_snapshot(VcsSnapshotKey { worktree_id, tier }, message)
        .await;
    metrics.snapshot_queued(coalesced);
}

async fn filter_workspace_worktree_ids(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    worktree_ids: Vec<WorktreeId>,
) -> Vec<WorktreeId> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for worktree_id in worktree_ids {
        if !seen.insert(worktree_id) {
            continue;
        }
        let Ok(store) = state.store_for_worktree(worktree_id).await else {
            continue;
        };
        let Ok(Some(worktree)) = store.get_worktree(worktree_id).await else {
            continue;
        };
        if worktree.workspace_id == workspace_id {
            out.push(worktree_id);
        }
    }
    out.sort_by_key(|worktree_id| worktree_id.0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_snapshot(worktree_id: WorktreeId, rev: i64) -> WorktreeVcsSnapshot {
        WorktreeVcsSnapshot {
            worktree_id,
            rev,
            emitted_at_ms: rev,
            base_commit_sha: "base".to_string(),
            head_commit_sha: "head".to_string(),
            target_branch: Some("origin/main".to_string()),
            target_branch_commit_sha: Some("target".to_string()),
            base_resolution: WorktreeVcsBaseResolution::default(),
            compute_state: WorktreeVcsComputeState::Ready,
            summary: WorktreeVcsSummary {
                file_count: Some(rev),
                line_additions: Some(rev),
                line_deletions: Some(0),
                line_count: Some(rev),
            },
            git_status: WorktreeVcsGitStatusSummary::default(),
            touched_files: WorktreeVcsTouchedFiles::default(),
            touched_files_state: WorktreeVcsTouchedFilesState::Ready,
            freshness: WorktreeVcsFreshness::Fresh,
            available: true,
            unavailable_reason: None,
            schema_version: 1,
        }
    }

    #[tokio::test]
    async fn pending_buffer_coalesces_ctx_ui_sized_vcs_storm_latest_wins() {
        let pending = Arc::new(VcsPendingBuffer::new());
        let metrics = Arc::new(VcsStreamMetrics::default());
        let workspace_id = WorkspaceId::new();
        let worktree_id = WorktreeId::new();
        let ctx_ui_vcs_event_count = 10_804;

        for rev in 1..=ctx_ui_vcs_event_count {
            queue_vcs_snapshot(
                &pending,
                &metrics,
                workspace_id,
                7,
                WorktreeVcsStreamTier::Summary,
                test_snapshot(worktree_id, rev),
            )
            .await;
        }

        let Some(message) = pending.pop().await else {
            panic!("expected latest VCS snapshot");
        };
        match message {
            WorktreeVcsStreamMessage::SummarySnapshot { snapshot, .. } => {
                assert_eq!(snapshot.worktree_id, worktree_id);
                assert_eq!(snapshot.rev, ctx_ui_vcs_event_count);
            }
            other => panic!("expected summary snapshot, got {other:?}"),
        }
        assert!(pending.is_empty().await);
        assert_eq!(
            metrics.snapshot_queued_count.load(AtomicOrdering::Relaxed),
            ctx_ui_vcs_event_count as u64
        );
        assert_eq!(
            metrics
                .snapshot_coalesced_count
                .load(AtomicOrdering::Relaxed),
            (ctx_ui_vcs_event_count - 1) as u64
        );
    }
}
