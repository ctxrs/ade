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
use crate::workspace_active_snapshot::{WorkspaceSessionReplay, WorkspaceSessionReplayItem};

use super::{MobileSecureEnvelope, MobileSecureStreamQuery, SecureEnvelope};
use queue::{
    filter_partial_delta_for_active_tasks, log_head_batch_push_error, push_stream_message,
    HeadBatchBuffer, StreamQueue,
};
use replay::{
    ensure_worktree_vcs_watchers_for_sessions, primary_session_id_for_active_task,
    queue_reset_required, queue_snapshot_payload, replay_session_events,
    resolve_workspace_active_snapshot_subscriptions, session_ids_for_active_task_summary,
    sync_active_worktrees, with_stream_rev, ReplayOutcome,
};

mod queue;
mod replay;
mod terminal;
mod workspace_active;

pub(super) use terminal::terminal_stream_ws;
#[cfg(test)]
use terminal::{queue_terminal_ws_message, TerminalWsQueueOutcome};
pub(super) use workspace_active::workspace_active_snapshot_stream_ws;

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
                                let replay = replay_session_events(
                                    &state,
                                    workspace_id,
                                    session_id,
                                    after_seq,
                                    "ctx_http.replay_session_events_secure.list",
                                    None,
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

const SESSION_REPLAY_MAX_EVENTS: usize = 2000;
const WORKSPACE_STREAM_QUEUE_LIMIT: usize = 256;
const WORKSPACE_STREAM_QUEUE_MAX_AGE: Duration = Duration::from_secs(10);
const HEAD_BATCH_FLUSH_INTERVAL: Duration = Duration::from_millis(25);
const HEAD_BATCH_SESSION_LIMIT: usize = 200;
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

pub(super) async fn dictation_livekit_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| crate::dictation_livekit::dictation_livekit_stream(socket, state))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[path = "ws_queue_tests.rs"]
    mod ws_queue_tests;
}
