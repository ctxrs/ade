use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::{Sink, SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinSet;
use tokio_tungstenite::{connect_async, tungstenite::Message as TungsteniteMessage};

use ctx_core::ids::*;
use ctx_core::models::*;

use crate::daemon::AppState;
use crate::git_status::emit_worktree_vcs_snapshot_for_worktree;
use crate::terminals::{TerminalClientMessage, TerminalServerMessage};
use crate::web_sessions::WebSessionManager;
use crate::workspace_active_snapshot::SessionReplayResult;

use super::{MobileSecureEnvelope, MobileSecureStreamQuery, SecureEnvelope};

pub(super) async fn mobile_secure_workspace_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<MobileSecureStreamQuery>,
) -> impl IntoResponse {
    let workspace_id = match uuid::Uuid::parse_str(&id) {
        Ok(v) => WorkspaceId(v),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let device_id = query.device_id.trim().to_string();
    ws.on_upgrade(move |socket| async move {
        if let Err(err) = handle_mobile_secure_ws(socket, state, workspace_id, device_id).await {
            tracing::warn!("secure mobile ws ended: {err:#}");
        }
    })
}

async fn handle_mobile_secure_ws(
    socket: WebSocket,
    state: Arc<AppState>,
    workspace_id: WorkspaceId,
    device_id: String,
) -> Result<(), anyhow::Error> {
    let (sender, mut receiver) = socket.split();
    let device_uuid = uuid::Uuid::parse_str(&device_id)?;
    let cfg = state.global_store().get_mobile_access_config().await?;
    let cfg = cfg.ok_or_else(|| anyhow::anyhow!("mobile access not configured"))?;
    let device = state
        .global_store()
        .get_mobile_device(MobileDeviceId(device_uuid))
        .await?
        .ok_or_else(|| anyhow::anyhow!("device not registered"))?;
    if device.profile_id != cfg.profile_id {
        return Err(anyhow::anyhow!("device not authorized for tunnel"));
    }
    let device_public_key = device
        .public_key
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("device missing public key"))?;
    let key =
        crate::mobile_e2ee::derive_key(&device_id, device_public_key, &cfg.daemon_private_key)?;

    let mut rx = state
        .workspaces
        .workspace_active_snapshot
        .subscribe(workspace_id)
        .await;
    let mut subscriptions: HashMap<SessionId, SessionCursor> = HashMap::new();
    let mut subscription_state = WorkspaceActiveSubscriptionState::default();
    let mut active_worktrees: HashSet<WorktreeId> = HashSet::new();
    let control = Arc::new(StreamQueue::new(
        WORKSPACE_STREAM_QUEUE_LIMIT,
        WORKSPACE_STREAM_QUEUE_MAX_AGE,
    ));
    let head_buffer = Arc::new(HeadBatchBuffer::new());
    let send_control = Arc::new(StreamSendControl::new());
    let mut reset_queued = false;

    let (snapshot_rev, archived_rev) =
        super::tasks::load_workspace_active_snapshot_state(&state, workspace_id).await;
    let ready = WorkspaceActiveSnapshotEvent::Ready {
        workspace_id,
        snapshot_rev,
        archived_rev,
    };
    if push_stream_message(
        &control,
        workspace_id,
        None,
        "ready_secure",
        WorkspaceActiveSnapshotStreamMessage::Event {
            rev: 0,
            event: Box::new(ready),
        },
    )
    .await
    .is_err()
    {
        return Ok(());
    }

    let latest_snapshot_rev = Arc::new(AtomicI64::new(snapshot_rev));
    let send_task = {
        let control = control.clone();
        let head_buffer = head_buffer.clone();
        let send_control = send_control.clone();
        let send_key = key.clone();
        let send_device_id = device_id.clone();
        let latest_snapshot_rev = latest_snapshot_rev.clone();
        tokio::spawn(async move {
            let mut sender = sender;
            let mut envelope_seq: i64 = 0;
            let mut stream_seq: i64 = 0;
            let mut tick = tokio::time::interval(HEAD_BATCH_FLUSH_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                if let Some(entry) = control.pop().await {
                    let message = entry.message;
                    let queued_ms = entry.enqueued_at.elapsed().as_millis();
                    let is_snapshot = matches!(
                        message,
                        WorkspaceActiveSnapshotStreamMessage::Snapshot { .. }
                    );
                    envelope_seq += 1;
                    let message = match message {
                        WorkspaceActiveSnapshotStreamMessage::ResetRequired { .. } => message,
                        other => {
                            stream_seq += 1;
                            with_stream_rev(other, stream_seq)
                        }
                    };
                    let send_start = Instant::now();
                    if send_secure_ws(
                        &mut sender,
                        &send_key,
                        &send_device_id,
                        envelope_seq,
                        &message,
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }
                    if is_snapshot {
                        let send_ms = send_start.elapsed().as_millis();
                        let payload_bytes = serde_json::to_vec(&message)
                            .map(|data| data.len())
                            .unwrap_or(0);
                        let (task_count, head_count) = match &message {
                            WorkspaceActiveSnapshotStreamMessage::Snapshot {
                                active_snapshot,
                                active_heads,
                                ..
                            } => (
                                active_snapshot.active.tasks.len(),
                                active_heads.as_ref().map(|h| h.heads.len()).unwrap_or(0),
                            ),
                            _ => (0, 0),
                        };
                        tracing::info!(
                            target: "ctx_http.ws_active_snapshot",
                            workspace_id = %workspace_id.0,
                            snapshot_bytes = payload_bytes,
                            snapshot_queue_ms = queued_ms,
                            snapshot_send_ms = send_ms,
                            active_tasks = task_count,
                            active_heads = head_count,
                            "workspace snapshot sent (secure)",
                        );
                    }
                    if is_snapshot {
                        send_control.clear_hydrating();
                    }
                    if send_control.should_disconnect_after_flush() && control.is_empty().await {
                        break;
                    }
                    continue;
                }
                if send_control.should_disconnect_after_flush() {
                    break;
                }

                if !send_control.is_hydrating() {
                    let (snapshot_rev, deltas) = head_buffer.take().await;
                    if !deltas.is_empty() {
                        let latest_rev = latest_snapshot_rev.load(Ordering::Relaxed);
                        let snapshot_rev = snapshot_rev.max(latest_rev);
                        bump_latest_snapshot_rev(&latest_snapshot_rev, snapshot_rev);
                        envelope_seq += 1;
                        stream_seq += 1;
                        let message = WorkspaceActiveSnapshotStreamMessage::HeadsBatch {
                            rev: stream_seq,
                            snapshot_rev,
                            deltas,
                        };
                        if send_secure_ws(
                            &mut sender,
                            &send_key,
                            &send_device_id,
                            envelope_seq,
                            &message,
                        )
                        .await
                        .is_err()
                        {
                            break;
                        }
                        continue;
                    }
                }

                tokio::select! {
                    _ = control.notify.notified() => {},
                    _ = head_buffer.notify.notified() => {},
                    _ = tick.tick() => {},
                }
            }
        })
    };
    let recv_loop = async {
        loop {
            tokio::select! {
                msg = receiver.next() => {
                    match msg {
                        Some(Ok(WsMessage::Text(text))) => {
                            let frame: MobileSecureEnvelope = match serde_json::from_str(&text) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };
                            let payload = crate::mobile_e2ee::decrypt(
                                &key,
                                &device_id,
                                frame.seq,
                                &frame.nonce,
                                &frame.ciphertext,
                            )?;
                            let message: WorkspaceActiveSnapshotClientMessage = serde_json::from_slice(&payload)?;
                            let include_active_heads = matches!(
                                &message,
                                WorkspaceActiveSnapshotClientMessage::Subscribe {
                                    include_active_heads: true,
                                    ..
                                }
                            );
                            state
                                .ensure_workspace_active_snapshot_hydrated(workspace_id)
                                .await;
                            let resolved = match resolve_workspace_active_snapshot_subscriptions(
                                &state,
                                workspace_id,
                                message,
                                &subscriptions,
                            )
                            .await
                            {
                                Ok(next) => next,
                                Err(_) => {
                                    tracing::error!(
                                        target: "ctx_http.ws_active_snapshot",
                                        workspace_id = %workspace_id.0,
                                        "workspace stream subscribe resolution failed (secure)",
                                    );
                                    control.clear().await;
                                    head_buffer.clear().await;
                                    if queue_reset_required(&control, &state, workspace_id)
                                        .await
                                        .is_err()
                                    {
                                        break;
                                    }
                                    reset_queued = true;
                                    send_control.set_disconnect_after_flush();
                                    continue;
                                }
                            };
                            let ResolvedWorkspaceActiveSubscriptions {
                                sessions: resolved_sessions,
                                state: next_state,
                            } = resolved;

                            control.clear().await;
                            head_buffer.clear().await;
                            reset_queued = false;
                            send_control.clear_disconnect_after_flush();
                            if include_active_heads {
                                send_control.set_hydrating();
                            }
                            if include_active_heads
                                && queue_snapshot_payload(&control, &state, workspace_id)
                                    .await
                                    .is_err()
                            {
                                break;
                            }

                            let mut skip_replay_sessions = HashSet::new();
                            if include_active_heads && next_state.active_scope {
                                for session_id in next_state.active_task_sessions.values() {
                                    skip_replay_sessions.insert(*session_id);
                                }
                            }

                            let mut next_map = HashMap::new();
                            let mut replay_failed = false;
                            for sub in &resolved_sessions {
                                let after_seq = sub.after_seq.unwrap_or(0);
                                let session_id = sub.session_id;
                                if include_active_heads
                                    && skip_replay_sessions.contains(&session_id)
                                {
                                    let last_sent = state.workspaces.workspace_active_snapshot
                                        .session_last_event_seq(workspace_id, session_id)
                                        .await
                                        .max(after_seq);
                                    next_map.insert(session_id, SessionCursor { last_sent });
                                    continue;
                                }
                                let control = control.clone();
                                let head_buffer = head_buffer.clone();
                                let active_task_sessions =
                                    next_state.active_task_sessions.clone();
                                let replay = replay_session_events_secure(
                                    &state,
                                    workspace_id,
                                    session_id,
                                    after_seq,
                                    move |event| {
                                        let control = control.clone();
                                        let head_buffer = head_buffer.clone();
                                        let active_task_sessions =
                                            active_task_sessions.clone();
                                        async move {
                                            match event {
                                                WorkspaceActiveSnapshotStreamMessage::Event {
                                                    event,
                                                    ..
                                                } => match *event {
                                                    WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                                                        snapshot_rev,
                                                        delta,
                                                        ..
                                                    } => {
                                                        let delta =
                                                            filter_partial_delta_for_active_tasks(
                                                                *delta,
                                                                &active_task_sessions,
                                                            );
                                                        if let Err(err) = head_buffer
                                                            .push(snapshot_rev, delta)
                                                            .await
                                                        {
                                                            log_head_batch_push_error(
                                                                "replay_secure",
                                                                workspace_id,
                                                                &err,
                                                            );
                                                            return Err(());
                                                        }
                                                        Ok(())
                                                    }
                                                    other => {
                                                        push_stream_message(
                                                            &control,
                                                            workspace_id,
                                                            Some(session_id),
                                                            "replay_secure",
                                                            WorkspaceActiveSnapshotStreamMessage::Event {
                                                                rev: 0,
                                                                event: Box::new(other),
                                                            },
                                                        )
                                                        .await
                                                    }
                                                },
                                                other => {
                                                    push_stream_message(
                                                        &control,
                                                        workspace_id,
                                                        Some(session_id),
                                                        "replay_secure",
                                                        other,
                                                    )
                                                    .await
                                                }
                                            }
                                        }
                                    },
                                )
                                .await;
                                match replay {
                                    Ok(ReplayOutcome::Replay { last_sent }) => {
                                        next_map.insert(session_id, SessionCursor { last_sent });
                                    }
                                    Ok(ReplayOutcome::ResetRequired) | Err(_) => {
                                        tracing::error!(
                                            target: "ctx_http.ws_active_snapshot",
                                            workspace_id = %workspace_id.0,
                                            session_id = %session_id.0,
                                            after_seq,
                                            "workspace stream replay failed (secure)",
                                        );
                                        replay_failed = true;
                                        break;
                                    }
                                };
                            }
                            if replay_failed {
                                control.clear().await;
                                head_buffer.clear().await;
                                if queue_reset_required(&control, &state, workspace_id)
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                reset_queued = true;
                                send_control.set_disconnect_after_flush();
                                continue;
                            }
                            subscriptions = next_map;
                            subscription_state = next_state;
                            let git_status_session_ids: Vec<SessionId> = resolved_sessions
                                .iter()
                                .map(|sub| sub.session_id)
                                .collect();
                            sync_active_worktrees(
                                &state,
                                &mut active_worktrees,
                                &git_status_session_ids,
                            )
                            .await;
                            ensure_worktree_vcs_watchers_for_sessions(
                                &state,
                                &git_status_session_ids,
                            )
                                .await;
                        }
                        Some(Ok(WsMessage::Close(_))) => break,
                        Some(Ok(_)) => {}
                        Some(Err(_)) => break,
                        None => break,
                    }
                }
                event = rx.recv() => {
                    let event = match event {
                        Ok(event) => event,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(lagged)) => {
                            if reset_queued {
                                continue;
                            }
                            tracing::error!(
                                target: "ctx_http.ws_active_snapshot",
                                workspace_id = %workspace_id.0,
                                lagged,
                                "workspace stream lagged (secure)",
                            );
                            control.clear().await;
                            head_buffer.clear().await;
                            if queue_reset_required(&control, &state, workspace_id)
                                .await
                                .is_err()
                            {
                                break;
                            }
                            reset_queued = true;
                            send_control.set_disconnect_after_flush();
                            continue;
                        }
                        Err(_) => break,
                    };

                    if let Some(rev) = event_snapshot_rev(&event) {
                        bump_latest_snapshot_rev(&latest_snapshot_rev, rev);
                    }

                    if reset_queued {
                        continue;
                    }

                    let mut refresh_active_worktrees = false;
                    if subscription_state.active_scope {
                        match &event {
                            WorkspaceActiveSnapshotEvent::ActiveTaskUpsert { task, .. } => {
                                let session_id = primary_session_id_for_active_task(task);
                                subscription_state
                                    .active_task_sessions
                                    .insert(task.task.id, session_id);
                                refresh_active_worktrees = true;
                                if let std::collections::hash_map::Entry::Vacant(entry) =
                                    subscriptions.entry(session_id)
                                {
                                    let last_sent = state.workspaces.workspace_active_snapshot
                                        .session_last_event_seq(workspace_id, session_id)
                                        .await;
                                    entry.insert(SessionCursor { last_sent });
                                }
                            }
                            WorkspaceActiveSnapshotEvent::ActiveTaskDelete { task_id, .. } => {
                                if let Some(session_id) =
                                    subscription_state.active_task_sessions.remove(task_id)
                                {
                                    refresh_active_worktrees = true;
                                    let still_active = subscription_state
                                        .active_task_sessions
                                        .values()
                                        .any(|id| *id == session_id);
                                    if !still_active
                                        && !subscription_state
                                            .explicit_sessions
                                            .contains(&session_id)
                                    {
                                        subscriptions.remove(&session_id);
                                    }
                                }
                            }
                            WorkspaceActiveSnapshotEvent::TaskDelta { delta, .. }
                                if matches!(delta.kind, TaskDeltaKind::Archived) =>
                            {
                                if let Some(session_id) =
                                    subscription_state.active_task_sessions.remove(&delta.task.id)
                                {
                                    refresh_active_worktrees = true;
                                    let still_active = subscription_state
                                        .active_task_sessions
                                        .values()
                                        .any(|id| *id == session_id);
                                    if !still_active
                                        && !subscription_state
                                            .explicit_sessions
                                            .contains(&session_id)
                                    {
                                        subscriptions.remove(&session_id);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    if refresh_active_worktrees {
                        let session_ids: Vec<SessionId> =
                            subscriptions.keys().copied().collect();
                        sync_active_worktrees(
                            &state,
                            &mut active_worktrees,
                            &session_ids,
                        )
                        .await;
                        ensure_worktree_vcs_watchers_for_sessions(
                            &state,
                            &session_ids,
                        )
                        .await;
                    }

                    if let Some(foreground_task_id) = subscription_state.foreground_task_id {
                        match &event {
                            WorkspaceActiveSnapshotEvent::ActiveTaskUpsert { task, .. }
                                if task.task.id == foreground_task_id =>
                            {
                                subscription_state.foreground_session_ids =
                                    Some(session_ids_for_active_task_summary(task));
                            }
                            WorkspaceActiveSnapshotEvent::ActiveTaskDelete { task_id, .. }
                                if *task_id == foreground_task_id =>
                            {
                                subscription_state.foreground_session_ids =
                                    Some(HashSet::new());
                            }
                            WorkspaceActiveSnapshotEvent::SessionSummary { summary, .. }
                                if summary.session.task_id == foreground_task_id =>
                            {
                                if let Some(ids) =
                                    subscription_state.foreground_session_ids.as_mut()
                                {
                                    ids.insert(summary.session.id);
                                }
                            }
                            WorkspaceActiveSnapshotEvent::SessionSummaryDelta { delta, .. }
                                if delta.task_id == foreground_task_id =>
                            {
                                if let Some(ids) =
                                    subscription_state.foreground_session_ids.as_mut()
                                {
                                    ids.insert(delta.session_id);
                                }
                            }
                            WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. }
                                if head.session.task_id == foreground_task_id =>
                            {
                                if let Some(ids) =
                                    subscription_state.foreground_session_ids.as_mut()
                                {
                                    ids.insert(head.session.id);
                                }
                            }
                            _ => {}
                        }
                    }

                    match &event {
                        WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => {
                            let Some(cursor) = subscriptions.get_mut(&delta.session_id) else {
                                continue;
                            };
                            if let Some(ev) = &delta.event {
                                if ev.seq >= 0 {
                                    if ev.seq <= cursor.last_sent {
                                        continue;
                                    }
                                    cursor.last_sent = ev.seq;
                                }
                            } else if delta.last_event_seq <= cursor.last_sent {
                                continue;
                            } else {
                                cursor.last_sent = delta.last_event_seq;
                            }
                        }
                        WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. } => {
                            let Some(cursor) = subscriptions.get_mut(&head.session.id) else {
                                continue;
                            };
                            if head.last_event_seq <= cursor.last_sent {
                                continue;
                            }
                            cursor.last_sent = head.last_event_seq;
                        }
                        _ => {}
                    }

                    let session_id = match &event {
                        WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => {
                            Some(delta.session_id)
                        }
                        WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. } => {
                            Some(head.session.id)
                        }
                        WorkspaceActiveSnapshotEvent::SessionGap { session_id, .. } => {
                            Some(*session_id)
                        }
                        WorkspaceActiveSnapshotEvent::SessionSummaryDelta { delta, .. } => {
                            Some(delta.session_id)
                        }
                        _ => None,
                    };
                    match event {
                        WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                            snapshot_rev,
                            delta,
                            ..
                        } => {
                            let delta = filter_partial_delta_for_active_tasks(
                                *delta,
                                &subscription_state.active_task_sessions,
                            );
                            if let Err(err) =
                                head_buffer.push(snapshot_rev, delta).await
                            {
                                log_head_batch_push_error(
                                    "event_secure",
                                    workspace_id,
                                    &err,
                                );
                                if reset_queued {
                                    continue;
                                }
                                control.clear().await;
                                head_buffer.clear().await;
                                if queue_reset_required(&control, &state, workspace_id)
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                reset_queued = true;
                                send_control.set_disconnect_after_flush();
                            }
                        }
                        other => {
                            if push_stream_message(
                                &control,
                                workspace_id,
                                session_id,
                                "event_secure",
                                WorkspaceActiveSnapshotStreamMessage::Event {
                                    rev: 0,
                                    event: Box::new(other),
                                },
                            )
                            .await
                            .is_err()
                            {
                                if reset_queued {
                                    continue;
                                }
                                control.clear().await;
                                head_buffer.clear().await;
                                if queue_reset_required(&control, &state, workspace_id)
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                reset_queued = true;
                                send_control.set_disconnect_after_flush();
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    };

    let (send_task, recv_result) = crate::async_util::race_join_handle(send_task, recv_loop).await;

    send_control.set_disconnect_after_flush();
    control.notify.notify_one();
    if let Some(send_task) = send_task {
        let _ = send_task.await;
    }

    state
        .update_worktree_vcs_activity(&active_worktrees, &HashSet::new())
        .await;

    recv_result.unwrap_or(Ok(()))
}

struct SessionCursor {
    last_sent: i64,
}

#[derive(Default)]
struct WorkspaceActiveSubscriptionState {
    active_scope: bool,
    explicit_sessions: HashSet<SessionId>,
    active_task_sessions: HashMap<TaskId, SessionId>,
    foreground_task_id: Option<TaskId>,
    foreground_session_ids: Option<HashSet<SessionId>>,
}

struct ResolvedWorkspaceActiveSubscriptions {
    sessions: Vec<WorkspaceActiveSnapshotSessionSubscription>,
    state: WorkspaceActiveSubscriptionState,
}

async fn send_secure_ws<S>(
    sink: &mut S,
    key: &crate::mobile_e2ee::E2eeKey,
    device_id: &str,
    seq: i64,
    payload: &WorkspaceActiveSnapshotStreamMessage,
) -> Result<(), anyhow::Error>
where
    S: Sink<WsMessage> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    let plaintext = serde_json::to_vec(payload)?;
    let envelope = crate::mobile_e2ee::encrypt(key, device_id, seq, &plaintext)?;
    let frame = SecureEnvelope {
        device_id: envelope.device_id,
        seq: envelope.seq,
        nonce: envelope.nonce_b64,
        ciphertext: envelope.ciphertext_b64,
    };
    let text = serde_json::to_string(&frame)?;
    sink.send(WsMessage::Text(text)).await?;
    Ok(())
}

fn terminal_stream_tail_bytes(params: &HashMap<String, String>) -> usize {
    params
        .get("tail")
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                trimmed.parse::<usize>().ok()
            }
        })
        .unwrap_or(crate::terminals::DEFAULT_OUTPUT_TAIL_BYTES)
}

fn bump_latest_snapshot_rev(latest: &AtomicI64, rev: i64) {
    let mut current = latest.load(Ordering::Relaxed);
    while rev > current {
        match latest.compare_exchange(current, rev, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
}

fn event_snapshot_rev(event: &WorkspaceActiveSnapshotEvent) -> Option<i64> {
    match event {
        WorkspaceActiveSnapshotEvent::Ready { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::ActiveTaskUpsert { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::ActiveTaskDelete { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::TaskDelta { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::SessionSummary { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::SessionSummaryDelta { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::SessionHeadDelta { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::SessionHeadSeed { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::SessionGap { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::WorktreeBootstrap { snapshot_rev, .. }
        | WorkspaceActiveSnapshotEvent::WorktreeVcsSnapshot { snapshot_rev, .. } => {
            Some(*snapshot_rev)
        }
        WorkspaceActiveSnapshotEvent::ArchivedTaskUpsert { .. }
        | WorkspaceActiveSnapshotEvent::ArchivedTaskDelete { .. } => None,
    }
}

pub(super) async fn terminal_stream_ws(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    let terminal_id = TerminalId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let session = state
        .transport
        .terminals
        .get(terminal_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    let tail_bytes = terminal_stream_tail_bytes(&params);
    Ok(ws.on_upgrade(move |socket| async move {
        handle_terminal_socket(socket, session, tail_bytes).await;
    }))
}

async fn handle_terminal_socket(
    mut socket: WebSocket,
    session: Arc<crate::terminals::TerminalSessionHandle>,
    snapshot_tail: usize,
) {
    session.mark_client_connected();
    let snapshot = session.snapshot();
    let status_payload = serde_json::to_string(&TerminalServerMessage::Status {
        status: snapshot.status.clone(),
        exit_code: snapshot.exit_code,
    })
    .unwrap_or_else(|_| "{\"type\":\"status\",\"status\":\"running\"}".to_string());
    let _ = socket.send(WsMessage::Text(status_payload)).await;

    let buffer = session.output_snapshot_tail(snapshot_tail);
    if !buffer.is_empty() {
        let _ = socket.send(WsMessage::Binary(buffer)).await;
    }

    let mut output_rx = session.output_receiver();
    let mut status_rx = session.status_receiver();

    let (mut ws_tx, mut ws_rx) = socket.split();
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<WsMessage>();
    let event_tx_output = event_tx.clone();
    let event_tx_status = event_tx.clone();
    let event_tx_input = event_tx.clone();
    let event_tx_ping = event_tx.clone();

    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        while let Some(msg) = event_rx.recv().await {
            if ws_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    tasks.spawn(async move {
        loop {
            match output_rx.recv().await {
                Ok(bytes) => {
                    let _ = event_tx_output.send(WsMessage::Binary(bytes));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    tasks.spawn(async move {
        loop {
            match status_rx.recv().await {
                Ok(ev) => {
                    let payload = serde_json::to_string(&TerminalServerMessage::Status {
                        status: ev.status,
                        exit_code: ev.exit_code,
                    })
                    .unwrap_or_else(|_| "{\"type\":\"status\",\"status\":\"exited\"}".to_string());
                    let _ = event_tx_status.send(WsMessage::Text(payload));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let session_input = session.clone();
    tasks.spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            match msg {
                WsMessage::Binary(data) => {
                    session_input.send_input(data);
                }
                WsMessage::Text(text) => {
                    if let Ok(parsed) = serde_json::from_str::<TerminalClientMessage>(&text) {
                        match parsed {
                            TerminalClientMessage::Resize { cols, rows } => {
                                let _ = session_input.resize(cols, rows);
                            }
                            TerminalClientMessage::Input { data } => {
                                session_input.send_input(data.into_bytes());
                            }
                            TerminalClientMessage::Ping => {
                                let payload = serde_json::to_string(&TerminalServerMessage::Pong)
                                    .unwrap_or_else(|_| "{\"type\":\"pong\"}".to_string());
                                let _ = event_tx_input.send(WsMessage::Text(payload));
                            }
                        }
                    } else {
                        session_input.send_input(text.into_bytes());
                    }
                }
                WsMessage::Close(_) => break,
                WsMessage::Ping(payload) => {
                    let _ = event_tx_input.send(WsMessage::Pong(payload));
                }
                WsMessage::Pong(_) => {}
            }
        }
    });

    tasks.spawn(async move {
        let mut interval = tokio::time::interval(TERMINAL_PING_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if event_tx_ping.send(WsMessage::Ping(Vec::new())).is_err() {
                break;
            }
        }
    });

    let _ = tasks.join_next().await;
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    session.mark_client_disconnected();
}

pub(super) async fn web_session_signal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    state
        .transport
        .web_sessions
        .get(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let manager = state.transport.web_sessions.clone();
    let session_id = id.clone();
    Ok(ws.on_upgrade(move |socket| async move {
        handle_web_session_socket(socket, manager, session_id).await;
    }))
}

async fn handle_web_session_socket(
    socket: WebSocket,
    manager: Arc<WebSessionManager>,
    session_id: String,
) {
    let handle = match manager.get(&session_id).await {
        Some(handle) => handle,
        None => {
            let _ = socket.close().await;
            return;
        }
    };
    let port = handle.worker_port().await;
    let url = format!("ws://127.0.0.1:{}/signal", port);
    let _ = manager.bump_viewers(&session_id, 1).await;

    let connect = connect_async(url).await;
    let upstream = match connect {
        Ok((stream, _)) => stream,
        Err(_) => {
            let _ = manager.bump_viewers(&session_id, -1).await;
            return;
        }
    };

    let (mut client_tx, mut client_rx) = socket.split();
    let (mut up_tx, mut up_rx) = upstream.split();

    let client_to_up = tokio::spawn(async move {
        while let Some(Ok(msg)) = client_rx.next().await {
            let out = match msg {
                WsMessage::Text(text) => TungsteniteMessage::Text(text.into()),
                WsMessage::Binary(bytes) => TungsteniteMessage::Binary(bytes.into()),
                WsMessage::Ping(bytes) => TungsteniteMessage::Ping(bytes.into()),
                WsMessage::Pong(bytes) => TungsteniteMessage::Pong(bytes.into()),
                WsMessage::Close(frame) => {
                    let frame =
                        frame.map(|f| tokio_tungstenite::tungstenite::protocol::CloseFrame {
                            code: f.code.into(),
                            reason: f.reason.to_string().into(),
                        });
                    TungsteniteMessage::Close(frame)
                }
            };
            if up_tx.send(out).await.is_err() {
                break;
            }
        }
    });

    let up_to_client = tokio::spawn(async move {
        while let Some(Ok(msg)) = up_rx.next().await {
            let out = match msg {
                TungsteniteMessage::Text(text) => WsMessage::Text(text.to_string()),
                TungsteniteMessage::Binary(bytes) => WsMessage::Binary(bytes.to_vec()),
                TungsteniteMessage::Ping(bytes) => WsMessage::Ping(bytes.to_vec()),
                TungsteniteMessage::Pong(bytes) => WsMessage::Pong(bytes.to_vec()),
                TungsteniteMessage::Close(frame) => {
                    let frame = frame.map(|f| axum::extract::ws::CloseFrame {
                        code: f.code.into(),
                        reason: f.reason.to_string().into(),
                    });
                    WsMessage::Close(frame)
                }
                TungsteniteMessage::Frame(_) => continue,
            };
            if client_tx.send(out).await.is_err() {
                break;
            }
        }
    });

    tokio::select! {
        _ = client_to_up => {},
        _ = up_to_client => {},
    };

    let _ = manager.bump_viewers(&session_id, -1).await;
}

pub(super) async fn workspace_active_snapshot_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workspace_id = match uuid::Uuid::parse_str(&id) {
        Ok(v) => WorkspaceId(v),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    ws.on_upgrade(move |socket| handle_workspace_active_snapshot_ws(socket, state, workspace_id))
}

const SESSION_REPLAY_MAX_EVENTS: usize = 2000;
const WORKSPACE_STREAM_QUEUE_LIMIT: usize = 256;
const WORKSPACE_STREAM_QUEUE_MAX_AGE: Duration = Duration::from_secs(10);
const HEAD_BATCH_FLUSH_INTERVAL: Duration = Duration::from_millis(25);
const HEAD_BATCH_SESSION_LIMIT: usize = 200;
const HEAD_BATCH_TOTAL_LIMIT: usize = 1000;
const TERMINAL_PING_INTERVAL: Duration = Duration::from_secs(25);

struct StreamQueueEntry<T> {
    enqueued_at: Instant,
    message: T,
}

#[derive(Debug)]
enum StreamQueuePushError {
    QueueFull { len: usize, limit: usize },
    QueueStale { age_ms: u64, max_age_ms: u64 },
}

struct StreamQueue<T> {
    pending: Mutex<VecDeque<StreamQueueEntry<T>>>,
    notify: Notify,
    limit: usize,
    max_age: Duration,
}

struct HeadBatchState {
    snapshot_rev: i64,
    total_len: usize,
    deltas: HashMap<SessionId, Vec<SessionHeadDelta>>,
}

struct HeadBatchBuffer {
    state: Mutex<HeadBatchState>,
    notify: Notify,
}

#[derive(Debug)]
enum HeadBatchPushError {
    SessionLimit { session_id: SessionId, limit: usize },
    TotalLimit { limit: usize },
}

fn is_partial_event(event: &SessionEvent) -> bool {
    matches!(
        event.event_type,
        SessionEventType::AssistantChunk | SessionEventType::ThoughtChunk
    )
}

fn allows_partial_for_active_primary_session(
    active_task_sessions: &HashMap<TaskId, SessionId>,
    session_id: SessionId,
) -> bool {
    active_task_sessions
        .values()
        .any(|active_session_id| *active_session_id == session_id)
}

fn filter_partial_delta_for_active_tasks(
    mut delta: SessionHeadDelta,
    active_task_sessions: &HashMap<TaskId, SessionId>,
) -> SessionHeadDelta {
    if let Some(event) = delta.event.as_ref() {
        if is_partial_event(event)
            && !allows_partial_for_active_primary_session(active_task_sessions, delta.session_id)
        {
            delta.event = None;
        }
    }
    delta
}

fn is_same_partial_type(prev: &SessionEvent, next: &SessionEvent) -> bool {
    matches!(
        (&prev.event_type, &next.event_type),
        (
            &SessionEventType::AssistantChunk,
            &SessionEventType::AssistantChunk
        ) | (
            &SessionEventType::ThoughtChunk,
            &SessionEventType::ThoughtChunk
        )
    )
}

fn extract_fragment(event: &SessionEvent) -> Option<&str> {
    event
        .payload_json
        .get("content_fragment")
        .and_then(Value::as_str)
}

fn merge_partial_fragment(prev: &str, next: &str) -> String {
    if prev.is_empty() {
        return next.to_string();
    }
    if next.is_empty() {
        return prev.to_string();
    }
    if next.starts_with(prev) {
        return next.to_string();
    }
    if prev.ends_with(next) {
        return prev.to_string();
    }
    format!("{prev}{next}")
}

fn try_coalesce_partial_delta(entry: &mut [SessionHeadDelta], next: &SessionHeadDelta) -> bool {
    let Some(prev) = entry.last_mut() else {
        return false;
    };
    if prev.turn.is_some()
        || prev.message.is_some()
        || next.turn.is_some()
        || next.message.is_some()
    {
        return false;
    }
    let (Some(prev_event), Some(next_event)) = (prev.event.as_ref(), next.event.as_ref()) else {
        return false;
    };
    if !is_partial_event(prev_event) || !is_partial_event(next_event) {
        return false;
    }
    if !is_same_partial_type(prev_event, next_event) {
        return false;
    }
    if prev_event.turn_id != next_event.turn_id || prev_event.turn_id.is_none() {
        return false;
    }
    let (Some(prev_fragment), Some(next_fragment)) =
        (extract_fragment(prev_event), extract_fragment(next_event))
    else {
        return false;
    };
    let merged_fragment = merge_partial_fragment(prev_fragment, next_fragment);
    let mut merged_event = next_event.clone();
    match merged_event.payload_json {
        Value::Object(ref mut map) => {
            map.insert(
                "content_fragment".to_string(),
                Value::String(merged_fragment),
            );
        }
        _ => return false,
    }
    prev.event = Some(merged_event);
    prev.last_event_seq = prev.last_event_seq.max(next.last_event_seq);
    prev.state_rev = prev.state_rev.max(next.state_rev);
    true
}

impl HeadBatchBuffer {
    fn new() -> Self {
        Self {
            state: Mutex::new(HeadBatchState {
                snapshot_rev: 0,
                total_len: 0,
                deltas: HashMap::new(),
            }),
            notify: Notify::new(),
        }
    }

    async fn push(
        &self,
        snapshot_rev: i64,
        delta: SessionHeadDelta,
    ) -> Result<(), HeadBatchPushError> {
        let mut state = self.state.lock().await;
        let session_id = delta.session_id;
        state.snapshot_rev = state.snapshot_rev.max(snapshot_rev);
        if let Some(entry) = state.deltas.get_mut(&session_id) {
            if try_coalesce_partial_delta(entry.as_mut_slice(), &delta) {
                self.notify.notify_one();
                return Ok(());
            }
        }
        if state.total_len >= HEAD_BATCH_TOTAL_LIMIT {
            return Err(HeadBatchPushError::TotalLimit {
                limit: HEAD_BATCH_TOTAL_LIMIT,
            });
        }
        {
            let entry = state.deltas.entry(session_id).or_default();
            if entry.len() >= HEAD_BATCH_SESSION_LIMIT {
                return Err(HeadBatchPushError::SessionLimit {
                    session_id,
                    limit: HEAD_BATCH_SESSION_LIMIT,
                });
            }
            entry.push(delta);
        }
        state.total_len += 1;
        self.notify.notify_one();
        Ok(())
    }

    async fn take(&self) -> (i64, Vec<SessionHeadDelta>) {
        let mut state = self.state.lock().await;
        if state.deltas.is_empty() {
            state.total_len = 0;
            return (state.snapshot_rev, Vec::new());
        }
        let snapshot_rev = state.snapshot_rev;
        let mut deltas = Vec::with_capacity(state.total_len);
        for (_, mut per_session) in state.deltas.drain() {
            deltas.append(&mut per_session);
        }
        state.total_len = 0;
        state.snapshot_rev = 0;
        (snapshot_rev, deltas)
    }

    async fn clear(&self) {
        let mut state = self.state.lock().await;
        state.deltas.clear();
        state.total_len = 0;
        state.snapshot_rev = 0;
    }
}

impl<T> StreamQueue<T> {
    fn new(limit: usize, max_age: Duration) -> Self {
        Self {
            pending: Mutex::new(VecDeque::new()),
            notify: Notify::new(),
            limit,
            max_age,
        }
    }

    async fn push(&self, message: T) -> Result<(), StreamQueuePushError> {
        let now = Instant::now();
        let mut guard = self.pending.lock().await;
        let len = guard.len();
        if len >= self.limit {
            return Err(StreamQueuePushError::QueueFull {
                len,
                limit: self.limit,
            });
        }
        if let Some(front) = guard.front() {
            let age = now.duration_since(front.enqueued_at);
            if age >= self.max_age {
                return Err(StreamQueuePushError::QueueStale {
                    age_ms: age.as_millis() as u64,
                    max_age_ms: self.max_age.as_millis() as u64,
                });
            }
        }
        guard.push_back(StreamQueueEntry {
            enqueued_at: now,
            message,
        });
        self.notify.notify_one();
        Ok(())
    }

    async fn clear(&self) {
        let mut guard = self.pending.lock().await;
        guard.clear();
    }

    async fn pop(&self) -> Option<StreamQueueEntry<T>> {
        let mut guard = self.pending.lock().await;
        guard.pop_front()
    }

    async fn is_empty(&self) -> bool {
        self.pending.lock().await.is_empty()
    }
}

fn log_stream_queue_push_error(
    context: &'static str,
    workspace_id: WorkspaceId,
    session_id: Option<SessionId>,
    err: &StreamQueuePushError,
) {
    match err {
        StreamQueuePushError::QueueFull { len, limit } => {
            tracing::error!(
                target: "ctx_http.ws_active_snapshot",
                workspace_id = %workspace_id.0,
                session_id = ?session_id,
                queue_len = *len,
                queue_limit = *limit,
                "workspace stream queue full ({context})",
            );
        }
        StreamQueuePushError::QueueStale { age_ms, max_age_ms } => {
            tracing::error!(
                target: "ctx_http.ws_active_snapshot",
                workspace_id = %workspace_id.0,
                session_id = ?session_id,
                oldest_age_ms = *age_ms,
                max_age_ms = *max_age_ms,
                "workspace stream queue stale ({context})",
            );
        }
    }
}

fn log_head_batch_push_error(
    context: &'static str,
    workspace_id: WorkspaceId,
    err: &HeadBatchPushError,
) {
    match err {
        HeadBatchPushError::SessionLimit { session_id, limit } => {
            tracing::error!(
                target: "ctx_http.ws_active_snapshot",
                workspace_id = %workspace_id.0,
                session_id = %session_id.0,
                limit = *limit,
                "workspace head batch session limit exceeded ({context})",
            );
        }
        HeadBatchPushError::TotalLimit { limit } => {
            tracing::error!(
                target: "ctx_http.ws_active_snapshot",
                workspace_id = %workspace_id.0,
                limit = *limit,
                "workspace head batch total limit exceeded ({context})",
            );
        }
    }
}

async fn push_stream_message(
    pending: &StreamQueue<WorkspaceActiveSnapshotStreamMessage>,
    workspace_id: WorkspaceId,
    session_id: Option<SessionId>,
    context: &'static str,
    message: WorkspaceActiveSnapshotStreamMessage,
) -> Result<(), ()> {
    match pending.push(message).await {
        Ok(()) => Ok(()),
        Err(err) => {
            log_stream_queue_push_error(context, workspace_id, session_id, &err);
            Err(())
        }
    }
}

struct StreamSendControl {
    disconnect_after_flush: AtomicBool,
    hydrating: AtomicBool,
}

impl StreamSendControl {
    fn new() -> Self {
        Self {
            disconnect_after_flush: AtomicBool::new(false),
            hydrating: AtomicBool::new(false),
        }
    }

    fn set_disconnect_after_flush(&self) {
        self.disconnect_after_flush.store(true, Ordering::Relaxed);
    }

    fn clear_disconnect_after_flush(&self) {
        self.disconnect_after_flush.store(false, Ordering::Relaxed);
    }

    fn should_disconnect_after_flush(&self) -> bool {
        self.disconnect_after_flush.load(Ordering::Relaxed)
    }

    fn set_hydrating(&self) {
        self.hydrating.store(true, Ordering::Relaxed);
    }

    fn clear_hydrating(&self) {
        self.hydrating.store(false, Ordering::Relaxed);
    }

    fn is_hydrating(&self) -> bool {
        self.hydrating.load(Ordering::Relaxed)
    }
}

enum ReplayOutcome {
    Replay { last_sent: i64 },
    ResetRequired,
}

fn with_stream_rev(
    message: WorkspaceActiveSnapshotStreamMessage,
    stream_rev: i64,
) -> WorkspaceActiveSnapshotStreamMessage {
    match message {
        WorkspaceActiveSnapshotStreamMessage::Snapshot {
            active_snapshot,
            active_heads,
            ..
        } => WorkspaceActiveSnapshotStreamMessage::Snapshot {
            rev: stream_rev,
            active_snapshot,
            active_heads,
        },
        WorkspaceActiveSnapshotStreamMessage::Event { event, .. } => {
            WorkspaceActiveSnapshotStreamMessage::Event {
                rev: stream_rev,
                event,
            }
        }
        WorkspaceActiveSnapshotStreamMessage::HeadsBatch {
            snapshot_rev,
            deltas,
            ..
        } => WorkspaceActiveSnapshotStreamMessage::HeadsBatch {
            rev: stream_rev,
            snapshot_rev,
            deltas,
        },
        WorkspaceActiveSnapshotStreamMessage::ResetRequired { latest_rev } => {
            WorkspaceActiveSnapshotStreamMessage::ResetRequired { latest_rev }
        }
    }
}

async fn queue_reset_required(
    pending: &StreamQueue<WorkspaceActiveSnapshotStreamMessage>,
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> Result<(), ()> {
    let (snapshot_rev, _) =
        super::tasks::load_workspace_active_snapshot_state(state, workspace_id).await;
    if crate::fault_injection::maybe_fail("ctx_http.send_workspace_active_reset").is_err() {
        return Err(());
    }
    push_stream_message(
        pending,
        workspace_id,
        None,
        "reset_required",
        WorkspaceActiveSnapshotStreamMessage::ResetRequired {
            latest_rev: snapshot_rev,
        },
    )
    .await
}

async fn queue_snapshot_payload(
    pending: &StreamQueue<WorkspaceActiveSnapshotStreamMessage>,
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> Result<(), ()> {
    let build_start = Instant::now();
    state
        .ensure_workspace_active_snapshot_hydrated(workspace_id)
        .await;
    let active_snapshot = state
        .workspaces
        .workspace_active_snapshot
        .active_snapshot(workspace_id, i64::MAX)
        .await;
    let active_heads = state
        .workspaces
        .workspace_active_snapshot
        .active_heads(workspace_id)
        .await;
    let snapshot_rev = active_snapshot.snapshot_rev;
    let task_count = active_snapshot.active.tasks.len();
    let head_count = active_heads.heads.len();
    let build_ms = build_start.elapsed().as_millis();
    if crate::fault_injection::maybe_fail("ctx_http.send_workspace_active_snapshot").is_err() {
        return Err(());
    }
    push_stream_message(
        pending,
        workspace_id,
        None,
        "snapshot",
        WorkspaceActiveSnapshotStreamMessage::Snapshot {
            rev: 0,
            active_snapshot,
            active_heads: Some(active_heads),
        },
    )
    .await?;
    tracing::info!(
        target: "ctx_http.ws_active_snapshot",
        workspace_id = %workspace_id.0,
        snapshot_rev,
        active_tasks = task_count,
        active_heads = head_count,
        snapshot_build_ms = build_ms,
        "workspace snapshot queued",
    );
    Ok(())
}

async fn replay_session_events_active<F, Fut>(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    after_seq: i64,
    mut emit: F,
) -> Result<ReplayOutcome, ()>
where
    F: FnMut(WorkspaceActiveSnapshotStreamMessage) -> Fut,
    Fut: std::future::Future<Output = Result<(), ()>>,
{
    let (snapshot_rev, _) =
        super::tasks::load_workspace_active_snapshot_state(state, workspace_id).await;
    if crate::fault_injection::maybe_fail("ctx_http.replay_session_events_active.list").is_err() {
        return Ok(ReplayOutcome::ResetRequired);
    }
    let replay = state
        .workspaces
        .workspace_active_snapshot
        .replay_session_head_deltas(
            workspace_id,
            session_id,
            after_seq,
            SESSION_REPLAY_MAX_EVENTS,
        )
        .await;
    match replay {
        SessionReplayResult::Replay { deltas, last_sent } => {
            for delta in deltas {
                let wrapped = WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                    workspace_id,
                    snapshot_rev,
                    delta: Box::new(delta),
                };
                crate::fault_injection::maybe_fail("ctx_http.replay_session_events_active.send")
                    .map_err(|_| ())?;
                emit(WorkspaceActiveSnapshotStreamMessage::Event {
                    rev: 0,
                    event: Box::new(wrapped),
                })
                .await?;
            }
            Ok(ReplayOutcome::Replay { last_sent })
        }
        SessionReplayResult::Gap {
            last_known_seq,
            reason,
        } => {
            // Treat `after_seq <= 0` as "not resuming": the client is not asking for replay from a
            // stable cursor. In that mode we avoid emitting `session_gap` noise and instead start
            // live updates from the current head.
            if after_seq <= 0 {
                return Ok(ReplayOutcome::Replay {
                    last_sent: last_known_seq.max(after_seq),
                });
            }
            let gap = WorkspaceActiveSnapshotEvent::SessionGap {
                workspace_id,
                snapshot_rev,
                session_id,
                after_seq,
                reason,
            };
            emit(WorkspaceActiveSnapshotStreamMessage::Event {
                rev: 0,
                event: Box::new(gap),
            })
            .await?;
            if let Some(head) = state
                .workspaces
                .workspace_active_snapshot
                .get_session_head(session_id)
                .await
            {
                let last_sent = head.last_event_seq.max(0);
                let seed = WorkspaceActiveSnapshotEvent::SessionHeadSeed {
                    workspace_id,
                    snapshot_rev,
                    head: Box::new(head),
                };
                emit(WorkspaceActiveSnapshotStreamMessage::Event {
                    rev: 0,
                    event: Box::new(seed),
                })
                .await?;
                return Ok(ReplayOutcome::Replay { last_sent });
            }
            Ok(ReplayOutcome::Replay {
                last_sent: last_known_seq.max(after_seq),
            })
        }
        SessionReplayResult::ResetRequired => Ok(ReplayOutcome::ResetRequired),
    }
}

fn primary_session_id_for_active_task(task: &WorkspaceActiveTaskSummary) -> SessionId {
    task.task
        .primary_session_id
        .unwrap_or(task.primary_session.session.id)
}

fn session_ids_for_active_task_summary(task: &WorkspaceActiveTaskSummary) -> HashSet<SessionId> {
    let mut sessions = HashSet::new();
    sessions.insert(task.primary_session.session.id);
    for summary in &task.sessions {
        sessions.insert(summary.session.id);
    }
    sessions
}

async fn resolve_foreground_task_sessions(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    task_id: TaskId,
) -> HashSet<SessionId> {
    if let Some(summary) = state
        .workspaces
        .workspace_active_snapshot
        .active_task_summary(workspace_id, task_id)
        .await
    {
        return session_ids_for_active_task_summary(&summary);
    }
    let store = match state.store_for_workspace(workspace_id).await {
        Ok(store) => store,
        Err(_) => return HashSet::new(),
    };
    match store.get_workspace_active_task_summary(task_id).await {
        Ok(Some(summary)) => session_ids_for_active_task_summary(&summary),
        _ => HashSet::new(),
    }
}

async fn resolve_workspace_active_snapshot_subscriptions(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    message: WorkspaceActiveSnapshotClientMessage,
    existing: &HashMap<SessionId, SessionCursor>,
) -> Result<ResolvedWorkspaceActiveSubscriptions, ()> {
    match message {
        WorkspaceActiveSnapshotClientMessage::Subscribe {
            session_ids,
            sessions,
            task_ids,
            foreground_task_id,
            scope,
            ..
        } => {
            let mut resolved = HashSet::new();
            let mut after_map: HashMap<SessionId, Option<i64>> = HashMap::new();
            let mut explicit_sessions = HashSet::new();
            let mut active_task_sessions = HashMap::new();
            let mut active_scope = false;
            let mut foreground_session_ids = None;
            for sub in sessions {
                after_map.insert(sub.session_id, sub.after_seq);
                resolved.insert(sub.session_id);
                explicit_sessions.insert(sub.session_id);
            }
            for session_id in session_ids {
                resolved.insert(session_id);
                explicit_sessions.insert(session_id);
            }
            if matches!(scope, Some(WorkspaceActiveSnapshotSubscribeScope::Active)) {
                active_scope = true;
                let snapshot = state
                    .workspaces
                    .workspace_active_snapshot
                    .active_snapshot(workspace_id, i64::MAX)
                    .await;
                for task in snapshot.active.tasks {
                    let session_id = primary_session_id_for_active_task(&task);
                    resolved.insert(session_id);
                    active_task_sessions.insert(task.task.id, session_id);
                }
            }
            if !task_ids.is_empty() {
                let store = state
                    .store_for_workspace(workspace_id)
                    .await
                    .map_err(|_| ())?;
                for task_id in task_ids {
                    let task = store.get_task(task_id).await.map_err(|_| ())?;
                    let Some(task) = task else {
                        continue;
                    };
                    if task.workspace_id != workspace_id {
                        continue;
                    }
                    if let Some(primary_session_id) = task.primary_session_id {
                        resolved.insert(primary_session_id);
                        explicit_sessions.insert(primary_session_id);
                    }
                }
            }
            if let Some(task_id) = foreground_task_id {
                let sessions = resolve_foreground_task_sessions(state, workspace_id, task_id).await;
                foreground_session_ids = Some(sessions);
            }

            let mut next: Vec<WorkspaceActiveSnapshotSessionSubscription> = resolved
                .into_iter()
                .map(|session_id| WorkspaceActiveSnapshotSessionSubscription {
                    session_id,
                    after_seq: after_map
                        .get(&session_id)
                        .copied()
                        .flatten()
                        .or_else(|| existing.get(&session_id).map(|cursor| cursor.last_sent)),
                })
                .collect();
            for sub in next.iter_mut() {
                if sub.after_seq.is_none() {
                    let last_seq = state
                        .workspaces
                        .workspace_active_snapshot
                        .session_last_event_seq(workspace_id, sub.session_id)
                        .await;
                    sub.after_seq = Some(last_seq);
                }
            }
            next.sort_by(|a, b| a.session_id.0.cmp(&b.session_id.0));
            Ok(ResolvedWorkspaceActiveSubscriptions {
                sessions: next,
                state: WorkspaceActiveSubscriptionState {
                    active_scope,
                    explicit_sessions,
                    active_task_sessions,
                    foreground_task_id,
                    foreground_session_ids,
                },
            })
        }
    }
}

async fn ensure_worktree_vcs_watchers_for_sessions(
    state: &Arc<AppState>,
    session_ids: &[SessionId],
) {
    if session_ids.is_empty() {
        return;
    }
    let mut worktrees: HashMap<WorktreeId, Worktree> = HashMap::new();
    for session_id in session_ids {
        let store = match state.store_for_session(*session_id).await {
            Ok(store) => store,
            Err(_) => continue,
        };
        let session = match store.get_session(*session_id).await {
            Ok(Some(session)) => session,
            _ => continue,
        };
        let worktree = match store.get_worktree(session.worktree_id).await {
            Ok(Some(worktree)) => worktree,
            _ => continue,
        };
        worktrees.entry(worktree.id).or_insert(worktree);
    }

    for (worktree_id, worktree) in worktrees {
        state.ensure_git_status_watcher(worktree.clone()).await;
        if let Err(err) = emit_worktree_vcs_snapshot_for_worktree(state, &worktree, true).await {
            tracing::warn!(worktree_id = %worktree_id.0, "worktree vcs snapshot failed: {err:#}");
        }
    }
}

async fn resolve_worktree_ids_for_sessions(
    state: &Arc<AppState>,
    session_ids: &[SessionId],
) -> HashSet<WorktreeId> {
    let mut worktree_ids = HashSet::new();
    for session_id in session_ids {
        let store = match state.store_for_session(*session_id).await {
            Ok(store) => store,
            Err(_) => continue,
        };
        let session = match store.get_session(*session_id).await {
            Ok(Some(session)) => session,
            _ => continue,
        };
        worktree_ids.insert(session.worktree_id);
    }
    worktree_ids
}

async fn sync_active_worktrees(
    state: &Arc<AppState>,
    active_worktrees: &mut HashSet<WorktreeId>,
    session_ids: &[SessionId],
) {
    let next = resolve_worktree_ids_for_sessions(state, session_ids).await;
    state
        .update_worktree_vcs_activity(active_worktrees, &next)
        .await;
    *active_worktrees = next;
}

async fn handle_workspace_active_snapshot_ws(
    socket: WebSocket,
    state: Arc<AppState>,
    workspace_id: WorkspaceId,
) {
    let (sender, mut receiver) = socket.split();
    let control = Arc::new(StreamQueue::new(
        WORKSPACE_STREAM_QUEUE_LIMIT,
        WORKSPACE_STREAM_QUEUE_MAX_AGE,
    ));
    let head_buffer = Arc::new(HeadBatchBuffer::new());
    let send_control = Arc::new(StreamSendControl::new());
    let mut rx = state
        .workspaces
        .workspace_active_snapshot
        .subscribe(workspace_id)
        .await;
    let mut subscriptions: HashMap<SessionId, SessionCursor> = HashMap::new();
    let mut subscription_state = WorkspaceActiveSubscriptionState::default();
    let mut active_worktrees: HashSet<WorktreeId> = HashSet::new();
    let mut reset_queued = false;

    let (snapshot_rev, archived_rev) =
        super::tasks::load_workspace_active_snapshot_state(&state, workspace_id).await;
    let ready = WorkspaceActiveSnapshotEvent::Ready {
        workspace_id,
        snapshot_rev,
        archived_rev,
    };
    if push_stream_message(
        &control,
        workspace_id,
        None,
        "ready",
        WorkspaceActiveSnapshotStreamMessage::Event {
            rev: 0,
            event: Box::new(ready),
        },
    )
    .await
    .is_err()
    {
        return;
    }

    let latest_snapshot_rev = Arc::new(AtomicI64::new(snapshot_rev));

    let send_task = {
        let control = control.clone();
        let head_buffer = head_buffer.clone();
        let send_control = send_control.clone();
        let latest_snapshot_rev = latest_snapshot_rev.clone();
        tokio::spawn(async move {
            let mut sender = sender;
            let mut stream_seq: i64 = 0;
            let mut tick = tokio::time::interval(HEAD_BATCH_FLUSH_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                if let Some(entry) = control.pop().await {
                    let message = entry.message;
                    let queued_ms = entry.enqueued_at.elapsed().as_millis();
                    let is_snapshot = matches!(
                        message,
                        WorkspaceActiveSnapshotStreamMessage::Snapshot { .. }
                    );
                    let message = match message {
                        WorkspaceActiveSnapshotStreamMessage::ResetRequired { .. } => message,
                        other => {
                            stream_seq += 1;
                            with_stream_rev(other, stream_seq)
                        }
                    };
                    let serialize_start = Instant::now();
                    let Ok(text) = serde_json::to_string(&message) else {
                        break;
                    };
                    let payload_bytes = text.len();
                    let send_start = Instant::now();
                    if sender.send(WsMessage::Text(text)).await.is_err() {
                        break;
                    }
                    if is_snapshot {
                        let encode_ms = serialize_start.elapsed().as_millis();
                        let send_ms = send_start.elapsed().as_millis();
                        let (task_count, head_count) = match &message {
                            WorkspaceActiveSnapshotStreamMessage::Snapshot {
                                active_snapshot,
                                active_heads,
                                ..
                            } => (
                                active_snapshot.active.tasks.len(),
                                active_heads.as_ref().map(|h| h.heads.len()).unwrap_or(0),
                            ),
                            _ => (0, 0),
                        };
                        tracing::info!(
                            target: "ctx_http.ws_active_snapshot",
                            workspace_id = %workspace_id.0,
                            snapshot_bytes = payload_bytes,
                            snapshot_queue_ms = queued_ms,
                            snapshot_encode_ms = encode_ms,
                            snapshot_send_ms = send_ms,
                            active_tasks = task_count,
                            active_heads = head_count,
                            "workspace snapshot sent",
                        );
                    }
                    if is_snapshot {
                        send_control.clear_hydrating();
                    }
                    if send_control.should_disconnect_after_flush() && control.is_empty().await {
                        break;
                    }
                    continue;
                }
                if send_control.should_disconnect_after_flush() {
                    break;
                }

                if !send_control.is_hydrating() {
                    let (snapshot_rev, deltas) = head_buffer.take().await;
                    if !deltas.is_empty() {
                        let latest_rev = latest_snapshot_rev.load(Ordering::Relaxed);
                        let snapshot_rev = snapshot_rev.max(latest_rev);
                        bump_latest_snapshot_rev(&latest_snapshot_rev, snapshot_rev);
                        stream_seq += 1;
                        let message = WorkspaceActiveSnapshotStreamMessage::HeadsBatch {
                            rev: stream_seq,
                            snapshot_rev,
                            deltas,
                        };
                        let Ok(text) = serde_json::to_string(&message) else {
                            break;
                        };
                        if sender.send(WsMessage::Text(text)).await.is_err() {
                            break;
                        }
                        continue;
                    }
                }

                tokio::select! {
                    _ = control.notify.notified() => {},
                    _ = head_buffer.notify.notified() => {},
                    _ = tick.tick() => {},
                }
            }
        })
    };
    let recv_loop = async {
        loop {
            tokio::select! {
                msg = receiver.next() => {
                    match msg {
                        Some(Ok(WsMessage::Text(text))) => {
                            if let Ok(message) =
                                serde_json::from_str::<WorkspaceActiveSnapshotClientMessage>(&text)
                            {
                                let include_active_heads = matches!(
                                    &message,
                                    WorkspaceActiveSnapshotClientMessage::Subscribe {
                                        include_active_heads: true,
                                        ..
                                    }
                                );
                                state
                                    .ensure_workspace_active_snapshot_hydrated(workspace_id)
                                    .await;
                                let resolved = match resolve_workspace_active_snapshot_subscriptions(
                                    &state,
                                    workspace_id,
                                    message,
                                    &subscriptions,
                                )
                                .await
                                {
                                    Ok(next) => next,
                                    Err(_) => {
                                        tracing::error!(
                                            target: "ctx_http.ws_active_snapshot",
                                            workspace_id = %workspace_id.0,
                                            "workspace stream subscribe resolution failed",
                                        );
                                        control.clear().await;
                                        head_buffer.clear().await;
                                        if queue_reset_required(&control, &state, workspace_id)
                                            .await
                                            .is_err()
                                        {
                                            break;
                                        }
                                        reset_queued = true;
                                        send_control.set_disconnect_after_flush();
                                        continue;
                                    }
                                };
                                let ResolvedWorkspaceActiveSubscriptions {
                                    sessions: resolved_sessions,
                                    state: next_state,
                                } = resolved;

                                control.clear().await;
                                head_buffer.clear().await;
                                reset_queued = false;
                                send_control.clear_disconnect_after_flush();
                                if include_active_heads {
                                    send_control.set_hydrating();
                                }
                                if include_active_heads
                                    && queue_snapshot_payload(&control, &state, workspace_id)
                                        .await
                                        .is_err()
                                {
                                    break;
                                }

                                let mut skip_replay_sessions = HashSet::new();
                                if include_active_heads && next_state.active_scope {
                                    for session_id in next_state.active_task_sessions.values() {
                                        skip_replay_sessions.insert(*session_id);
                                    }
                                }

                                let mut next_map = HashMap::new();
                                let mut replay_failed = false;
                                for sub in &resolved_sessions {
                                    let after_seq = sub.after_seq.unwrap_or(0);
                                    let session_id = sub.session_id;
                                    if include_active_heads
                                        && skip_replay_sessions.contains(&session_id)
                                    {
                                        let last_sent = state.workspaces.workspace_active_snapshot
                                            .session_last_event_seq(workspace_id, session_id)
                                            .await
                                            .max(after_seq);
                                        next_map.insert(session_id, SessionCursor { last_sent });
                                        continue;
                                    }
                                    let control = control.clone();
                                    let head_buffer = head_buffer.clone();
                                    let active_task_sessions =
                                        next_state.active_task_sessions.clone();
                                    let replay = replay_session_events_active(
                                        &state,
                                        workspace_id,
                                        session_id,
                                        after_seq,
                                        move |event| {
                                            let control = control.clone();
                                            let head_buffer = head_buffer.clone();
                                            let active_task_sessions =
                                                active_task_sessions.clone();
                                            async move {
                                                match event {
                                                    WorkspaceActiveSnapshotStreamMessage::Event {
                                                        event,
                                                        ..
                                                    } => match *event {
                                                        WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                                                            snapshot_rev,
                                                            delta,
                                                            ..
                                                        } => {
                                                            let delta =
                                                                filter_partial_delta_for_active_tasks(
                                                                    *delta,
                                                                    &active_task_sessions,
                                                                );
                                                            if let Err(err) = head_buffer
                                                                .push(snapshot_rev, delta)
                                                                .await
                                                            {
                                                                log_head_batch_push_error(
                                                                    "replay",
                                                                    workspace_id,
                                                                    &err,
                                                                );
                                                                return Err(());
                                                            }
                                                            Ok(())
                                                        }
                                                        other => {
                                                            push_stream_message(
                                                                &control,
                                                                workspace_id,
                                                                Some(session_id),
                                                                "replay",
                                                                WorkspaceActiveSnapshotStreamMessage::Event {
                                                                    rev: 0,
                                                                    event: Box::new(other),
                                                                },
                                                            )
                                                            .await
                                                        }
                                                    },
                                                    other => {
                                                        push_stream_message(
                                                            &control,
                                                            workspace_id,
                                                            Some(session_id),
                                                            "replay",
                                                            other,
                                                        )
                                                        .await
                                                    }
                                                }
                                            }
                                        },
                                    )
                                    .await;
                                    match replay {
                                        Ok(ReplayOutcome::Replay { last_sent }) => {
                                            next_map.insert(session_id, SessionCursor { last_sent });
                                        }
                                        Ok(ReplayOutcome::ResetRequired) | Err(_) => {
                                            tracing::error!(
                                                target: "ctx_http.ws_active_snapshot",
                                                workspace_id = %workspace_id.0,
                                                session_id = %session_id.0,
                                                after_seq,
                                                "workspace stream replay failed",
                                            );
                                            replay_failed = true;
                                            break;
                                        }
                                    };
                                }
                                if replay_failed {
                                    control.clear().await;
                                    head_buffer.clear().await;
                                    if queue_reset_required(&control, &state, workspace_id)
                                        .await
                                        .is_err()
                                    {
                                        break;
                                    }
                                    reset_queued = true;
                                    send_control.set_disconnect_after_flush();
                                    continue;
                                }
                                subscriptions = next_map;
                                subscription_state = next_state;
                                let git_status_session_ids: Vec<SessionId> = resolved_sessions
                                    .iter()
                                    .map(|sub| sub.session_id)
                                    .collect();
                                sync_active_worktrees(
                                    &state,
                                    &mut active_worktrees,
                                    &git_status_session_ids,
                                )
                                .await;
                                ensure_worktree_vcs_watchers_for_sessions(
                                    &state,
                                    &git_status_session_ids,
                                )
                                    .await;
                            }
                        }
                        Some(Ok(WsMessage::Binary(bytes))) => {
                            if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                                if let Ok(message) =
                                    serde_json::from_str::<WorkspaceActiveSnapshotClientMessage>(&text)
                                {
                                    let include_active_heads = matches!(
                                        &message,
                                        WorkspaceActiveSnapshotClientMessage::Subscribe {
                                            include_active_heads: true,
                                            ..
                                        }
                                    );
                                    state
                                        .ensure_workspace_active_snapshot_hydrated(workspace_id)
                                        .await;
                                    let resolved = match resolve_workspace_active_snapshot_subscriptions(
                                        &state,
                                        workspace_id,
                                        message,
                                        &subscriptions,
                                    )
                                    .await
                                    {
                                        Ok(next) => next,
                                        Err(_) => {
                                            tracing::error!(
                                                target: "ctx_http.ws_active_snapshot",
                                                workspace_id = %workspace_id.0,
                                                "workspace stream subscribe resolution failed (binary)",
                                            );
                                            control.clear().await;
                                            head_buffer.clear().await;
                                            if queue_reset_required(&control, &state, workspace_id)
                                                .await
                                                .is_err()
                                            {
                                                break;
                                            }
                                            reset_queued = true;
                                            send_control.set_disconnect_after_flush();
                                            continue;
                                        }
                                    };
                                    let ResolvedWorkspaceActiveSubscriptions {
                                        sessions: resolved_sessions,
                                        state: next_state,
                                    } = resolved;

                                    control.clear().await;
                                    head_buffer.clear().await;
                                    reset_queued = false;
                                    send_control.clear_disconnect_after_flush();
                                    if include_active_heads {
                                        send_control.set_hydrating();
                                    }
                                    if include_active_heads
                                        && queue_snapshot_payload(&control, &state, workspace_id)
                                            .await
                                            .is_err()
                                    {
                                        break;
                                    }

                                    let mut skip_replay_sessions = HashSet::new();
                                    if include_active_heads && next_state.active_scope {
                                        for session_id in next_state.active_task_sessions.values() {
                                            skip_replay_sessions.insert(*session_id);
                                        }
                                    }

                                    let mut next_map = HashMap::new();
                                    let mut replay_failed = false;
                                    for sub in &resolved_sessions {
                                        let after_seq = sub.after_seq.unwrap_or(0);
                                        let session_id = sub.session_id;
                                        if include_active_heads
                                            && skip_replay_sessions.contains(&session_id)
                                        {
                                            let last_sent = state.workspaces.workspace_active_snapshot
                                                .session_last_event_seq(workspace_id, session_id)
                                                .await
                                                .max(after_seq);
                                            next_map.insert(session_id, SessionCursor { last_sent });
                                            continue;
                                        }
                                        let control = control.clone();
                                        let head_buffer = head_buffer.clone();
                                        let active_task_sessions =
                                            next_state.active_task_sessions.clone();
                                        let replay = replay_session_events_active(
                                            &state,
                                            workspace_id,
                                            session_id,
                                            after_seq,
                                            move |event| {
                                                let control = control.clone();
                                                let head_buffer = head_buffer.clone();
                                                let active_task_sessions =
                                                    active_task_sessions.clone();
                                                async move {
                                                    match event {
                                                        WorkspaceActiveSnapshotStreamMessage::Event {
                                                            event,
                                                            ..
                                                        } => match *event {
                                                            WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                                                                snapshot_rev,
                                                                delta,
                                                                ..
                                                            } => {
                                                                let delta =
                                                                    filter_partial_delta_for_active_tasks(
                                                                        *delta,
                                                                        &active_task_sessions,
                                                                    );
                                                                if let Err(err) = head_buffer
                                                                    .push(snapshot_rev, delta)
                                                                    .await
                                                                {
                                                                    log_head_batch_push_error(
                                                                        "replay_binary",
                                                                        workspace_id,
                                                                        &err,
                                                                    );
                                                                    return Err(());
                                                                }
                                                                Ok(())
                                                            }
                                                            other => {
                                                                push_stream_message(
                                                                    &control,
                                                                    workspace_id,
                                                                    Some(session_id),
                                                                    "replay_binary",
                                                                    WorkspaceActiveSnapshotStreamMessage::Event {
                                                                        rev: 0,
                                                                        event: Box::new(other),
                                                                    },
                                                                )
                                                                .await
                                                            }
                                                        },
                                                        other => {
                                                            push_stream_message(
                                                                &control,
                                                                workspace_id,
                                                                Some(session_id),
                                                                "replay_binary",
                                                                other,
                                                            )
                                                            .await
                                                        }
                                                    }
                                                }
                                            },
                                        )
                                        .await;
                                        match replay {
                                            Ok(ReplayOutcome::Replay { last_sent }) => {
                                                next_map.insert(session_id, SessionCursor { last_sent });
                                            }
                                            Ok(ReplayOutcome::ResetRequired) | Err(_) => {
                                                tracing::error!(
                                                    target: "ctx_http.ws_active_snapshot",
                                                    workspace_id = %workspace_id.0,
                                                    session_id = %session_id.0,
                                                    after_seq,
                                                    "workspace stream replay failed (binary)",
                                                );
                                                replay_failed = true;
                                                break;
                                            }
                                        };
                                    }
                                    if replay_failed {
                                        control.clear().await;
                                        head_buffer.clear().await;
                                        if queue_reset_required(&control, &state, workspace_id)
                                            .await
                                            .is_err()
                                        {
                                            break;
                                        }
                                        reset_queued = true;
                                        send_control.set_disconnect_after_flush();
                                        continue;
                                    }
                                subscriptions = next_map;
                                subscription_state = next_state;
                                let git_status_session_ids: Vec<SessionId> = resolved_sessions
                                    .iter()
                                    .map(|sub| sub.session_id)
                                    .collect();
                                sync_active_worktrees(
                                    &state,
                                    &mut active_worktrees,
                                    &git_status_session_ids,
                                )
                                .await;
                                ensure_worktree_vcs_watchers_for_sessions(
                                    &state,
                                    &git_status_session_ids,
                                )
                                    .await;
                                }
                            }
                        }
                        Some(Ok(WsMessage::Close(_))) => break,
                        Some(Ok(_)) => {}
                        Some(Err(_)) => break,
                        None => break,
                    }
                }
                event = rx.recv() => {
                    let event = match event {
                        Ok(event) => event,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(lagged)) => {
                            if reset_queued {
                                continue;
                            }
                            tracing::error!(
                                target: "ctx_http.ws_active_snapshot",
                                workspace_id = %workspace_id.0,
                                lagged,
                                "workspace stream lagged",
                            );
                            control.clear().await;
                            head_buffer.clear().await;
                            if queue_reset_required(&control, &state, workspace_id)
                                .await
                                .is_err()
                            {
                                break;
                            }
                            reset_queued = true;
                            send_control.set_disconnect_after_flush();
                            continue;
                        }
                        Err(_) => break,
                    };

                    if let Some(rev) = event_snapshot_rev(&event) {
                        bump_latest_snapshot_rev(&latest_snapshot_rev, rev);
                    }

                    if reset_queued {
                        continue;
                    }

                    let mut refresh_active_worktrees = false;
                    if subscription_state.active_scope {
                        match &event {
                            WorkspaceActiveSnapshotEvent::ActiveTaskUpsert { task, .. } => {
                                let session_id = primary_session_id_for_active_task(task);
                                subscription_state
                                    .active_task_sessions
                                    .insert(task.task.id, session_id);
                                refresh_active_worktrees = true;
                                if let std::collections::hash_map::Entry::Vacant(entry) =
                                    subscriptions.entry(session_id)
                                {
                                    let last_sent = state.workspaces.workspace_active_snapshot
                                        .session_last_event_seq(workspace_id, session_id)
                                        .await;
                                    entry.insert(SessionCursor { last_sent });
                                }
                            }
                            WorkspaceActiveSnapshotEvent::ActiveTaskDelete { task_id, .. } => {
                                if let Some(session_id) =
                                    subscription_state.active_task_sessions.remove(task_id)
                                {
                                    refresh_active_worktrees = true;
                                    let still_active = subscription_state
                                        .active_task_sessions
                                        .values()
                                        .any(|id| *id == session_id);
                                    if !still_active
                                        && !subscription_state
                                            .explicit_sessions
                                            .contains(&session_id)
                                    {
                                        subscriptions.remove(&session_id);
                                    }
                                }
                            }
                            WorkspaceActiveSnapshotEvent::TaskDelta { delta, .. }
                                if matches!(delta.kind, TaskDeltaKind::Archived) =>
                            {
                                if let Some(session_id) =
                                    subscription_state.active_task_sessions.remove(&delta.task.id)
                                {
                                    refresh_active_worktrees = true;
                                    let still_active = subscription_state
                                        .active_task_sessions
                                        .values()
                                        .any(|id| *id == session_id);
                                    if !still_active
                                        && !subscription_state
                                            .explicit_sessions
                                            .contains(&session_id)
                                    {
                                        subscriptions.remove(&session_id);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    if refresh_active_worktrees {
                        let session_ids: Vec<SessionId> =
                            subscriptions.keys().copied().collect();
                        sync_active_worktrees(
                            &state,
                            &mut active_worktrees,
                            &session_ids,
                        )
                        .await;
                        ensure_worktree_vcs_watchers_for_sessions(
                            &state,
                            &session_ids,
                        )
                        .await;
                    }

                    if let Some(foreground_task_id) = subscription_state.foreground_task_id {
                        match &event {
                            WorkspaceActiveSnapshotEvent::ActiveTaskUpsert { task, .. }
                                if task.task.id == foreground_task_id =>
                            {
                                subscription_state.foreground_session_ids =
                                    Some(session_ids_for_active_task_summary(task));
                            }
                            WorkspaceActiveSnapshotEvent::ActiveTaskDelete { task_id, .. }
                                if *task_id == foreground_task_id =>
                            {
                                subscription_state.foreground_session_ids =
                                    Some(HashSet::new());
                            }
                            WorkspaceActiveSnapshotEvent::SessionSummary { summary, .. }
                                if summary.session.task_id == foreground_task_id =>
                            {
                                if let Some(ids) =
                                    subscription_state.foreground_session_ids.as_mut()
                                {
                                    ids.insert(summary.session.id);
                                }
                            }
                            WorkspaceActiveSnapshotEvent::SessionSummaryDelta { delta, .. }
                                if delta.task_id == foreground_task_id =>
                            {
                                if let Some(ids) =
                                    subscription_state.foreground_session_ids.as_mut()
                                {
                                    ids.insert(delta.session_id);
                                }
                            }
                            WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. }
                                if head.session.task_id == foreground_task_id =>
                            {
                                if let Some(ids) =
                                    subscription_state.foreground_session_ids.as_mut()
                                {
                                    ids.insert(head.session.id);
                                }
                            }
                            _ => {}
                        }
                    }

                    match &event {
                        WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => {
                            let Some(cursor) = subscriptions.get_mut(&delta.session_id) else {
                                continue;
                            };
                            if let Some(ev) = &delta.event {
                                if ev.seq >= 0 {
                                    if ev.seq <= cursor.last_sent {
                                        continue;
                                    }
                                    cursor.last_sent = ev.seq;
                                }
                            } else if delta.last_event_seq <= cursor.last_sent {
                                continue;
                            } else {
                                cursor.last_sent = delta.last_event_seq;
                            }
                        }
                        WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. } => {
                            let Some(cursor) = subscriptions.get_mut(&head.session.id) else {
                                continue;
                            };
                            if head.last_event_seq <= cursor.last_sent {
                                continue;
                            }
                            cursor.last_sent = head.last_event_seq;
                        }
                        _ => {}
                    }

                    let session_id = match &event {
                        WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => {
                            Some(delta.session_id)
                        }
                        WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. } => {
                            Some(head.session.id)
                        }
                        WorkspaceActiveSnapshotEvent::SessionGap { session_id, .. } => {
                            Some(*session_id)
                        }
                        WorkspaceActiveSnapshotEvent::SessionSummaryDelta { delta, .. } => {
                            Some(delta.session_id)
                        }
                        _ => None,
                    };
                    match event {
                        WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                            snapshot_rev,
                            delta,
                            ..
                        } => {
                            let delta = filter_partial_delta_for_active_tasks(
                                *delta,
                                &subscription_state.active_task_sessions,
                            );
                            if let Err(err) =
                                head_buffer.push(snapshot_rev, delta).await
                            {
                                log_head_batch_push_error(
                                    "event",
                                    workspace_id,
                                    &err,
                                );
                                if reset_queued {
                                    continue;
                                }
                                control.clear().await;
                                head_buffer.clear().await;
                                if queue_reset_required(&control, &state, workspace_id)
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                reset_queued = true;
                                send_control.set_disconnect_after_flush();
                            }
                        }
                        other => {
                            if push_stream_message(
                                &control,
                                workspace_id,
                                session_id,
                                "event",
                                WorkspaceActiveSnapshotStreamMessage::Event {
                                    rev: 0,
                                    event: Box::new(other),
                                },
                            )
                            .await
                            .is_err()
                            {
                                if reset_queued {
                                    continue;
                                }
                                control.clear().await;
                                head_buffer.clear().await;
                                if queue_reset_required(&control, &state, workspace_id)
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                reset_queued = true;
                                send_control.set_disconnect_after_flush();
                            }
                        }
                    }
                }
            }
        }
    };

    let (send_task, _recv_result) = crate::async_util::race_join_handle(send_task, recv_loop).await;

    send_control.set_disconnect_after_flush();
    control.notify.notify_one();
    if let Some(send_task) = send_task {
        let _ = send_task.await;
    }

    state
        .update_worktree_vcs_activity(&active_worktrees, &HashSet::new())
        .await;
}

async fn replay_session_events_secure<F, Fut>(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    after_seq: i64,
    mut emit: F,
) -> Result<ReplayOutcome, ()>
where
    F: FnMut(WorkspaceActiveSnapshotStreamMessage) -> Fut,
    Fut: std::future::Future<Output = Result<(), ()>>,
{
    let (snapshot_rev, _) =
        super::tasks::load_workspace_active_snapshot_state(state, workspace_id).await;
    if crate::fault_injection::maybe_fail("ctx_http.replay_session_events_secure.list").is_err() {
        return Ok(ReplayOutcome::ResetRequired);
    }
    let replay = state
        .workspaces
        .workspace_active_snapshot
        .replay_session_head_deltas(
            workspace_id,
            session_id,
            after_seq,
            SESSION_REPLAY_MAX_EVENTS,
        )
        .await;
    match replay {
        SessionReplayResult::Replay { deltas, last_sent } => {
            for delta in deltas {
                let wrapped = WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                    workspace_id,
                    snapshot_rev,
                    delta: Box::new(delta),
                };
                emit(WorkspaceActiveSnapshotStreamMessage::Event {
                    rev: 0,
                    event: Box::new(wrapped),
                })
                .await?;
            }
            Ok(ReplayOutcome::Replay { last_sent })
        }
        SessionReplayResult::Gap {
            last_known_seq,
            reason,
        } => {
            if after_seq <= 0 {
                return Ok(ReplayOutcome::Replay {
                    last_sent: last_known_seq.max(after_seq),
                });
            }
            let gap = WorkspaceActiveSnapshotEvent::SessionGap {
                workspace_id,
                snapshot_rev,
                session_id,
                after_seq,
                reason,
            };
            emit(WorkspaceActiveSnapshotStreamMessage::Event {
                rev: 0,
                event: Box::new(gap),
            })
            .await?;
            if let Some(head) = state
                .workspaces
                .workspace_active_snapshot
                .get_session_head(session_id)
                .await
            {
                let last_sent = head.last_event_seq.max(0);
                let seed = WorkspaceActiveSnapshotEvent::SessionHeadSeed {
                    workspace_id,
                    snapshot_rev,
                    head: Box::new(head),
                };
                emit(WorkspaceActiveSnapshotStreamMessage::Event {
                    rev: 0,
                    event: Box::new(seed),
                })
                .await?;
                return Ok(ReplayOutcome::Replay { last_sent });
            }
            Ok(ReplayOutcome::Replay {
                last_sent: last_known_seq.max(after_seq),
            })
        }
        SessionReplayResult::ResetRequired => Ok(ReplayOutcome::ResetRequired),
    }
}

pub(super) async fn dictation_livekit_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| crate::dictation_livekit::dictation_livekit_stream(socket, state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;
    use std::collections::HashMap;

    fn make_partial_delta(
        session_id: SessionId,
        turn_id: TurnId,
        fragment: &str,
    ) -> SessionHeadDelta {
        let event = SessionEvent {
            seq: -1,
            id: SessionEventId::new(),
            session_id,
            run_id: None,
            turn_id: Some(turn_id),
            event_type: SessionEventType::AssistantChunk,
            payload_json: json!({ "content_fragment": fragment }),
            transient: true,
            created_at: Utc::now(),
        };
        SessionHeadDelta {
            session_id,
            last_event_seq: 0,
            state_rev: 0,
            event: Some(event),
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        }
    }

    fn fragment_from_delta(delta: &SessionHeadDelta) -> String {
        delta
            .event
            .as_ref()
            .and_then(|event| event.payload_json.get("content_fragment"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }

    #[tokio::test]
    async fn head_batch_coalesces_partials_per_session() {
        let buffer = HeadBatchBuffer::new();
        let session_a = SessionId::new();
        let session_b = SessionId::new();
        let turn_a = TurnId::new();
        let turn_b = TurnId::new();
        let limit = HEAD_BATCH_SESSION_LIMIT + 25;

        for i in 0..limit {
            let fragment = format!("a-{i}-");
            buffer
                .push(1, make_partial_delta(session_a, turn_a, &fragment))
                .await
                .expect("partial burst should coalesce");
            if i % 5 == 0 {
                let fragment_b = format!("b-{i}-");
                buffer
                    .push(1, make_partial_delta(session_b, turn_b, &fragment_b))
                    .await
                    .expect("partial burst should coalesce");
            }
        }

        let (_, deltas) = buffer.take().await;
        let mut by_session = HashMap::new();
        for delta in deltas {
            by_session.insert(delta.session_id, delta);
        }
        assert_eq!(by_session.len(), 2);

        let merged_a = fragment_from_delta(by_session.get(&session_a).unwrap());
        assert!(merged_a.contains("a-0-"));
        assert!(merged_a.contains(&format!("a-{}-", limit - 1)));
    }
}
