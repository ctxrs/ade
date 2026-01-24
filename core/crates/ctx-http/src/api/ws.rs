use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::{Sink, SinkExt, StreamExt};
use serde::Serialize;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinSet;
use tokio_tungstenite::{connect_async, tungstenite::Message as TungsteniteMessage};

use ctx_core::ids::*;
use ctx_core::models::*;

use crate::daemon::AppState;
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
        .workspace_active_snapshot
        .subscribe(workspace_id)
        .await;
    let mut subscriptions: HashMap<SessionId, SessionCursor> = HashMap::new();
    let pending = Arc::new(StreamQueue::new(
        WORKSPACE_STREAM_QUEUE_LIMIT,
        WORKSPACE_STREAM_QUEUE_MAX_AGE,
    ));
    let send_control = Arc::new(StreamSendControl::new());
    let mut reset_queued = false;

    let (snapshot_rev, archived_rev) =
        super::load_workspace_active_snapshot_state(&state, workspace_id).await;
    let ready = WorkspaceActiveSnapshotEvent::Ready {
        workspace_id,
        snapshot_rev,
        archived_rev,
    };
    if pending
        .push(WorkspaceActiveSnapshotWsPayload::Event(ready))
        .await
        .is_err()
    {
        return Ok(());
    }

    let send_task = {
        let pending = pending.clone();
        let send_control = send_control.clone();
        let send_key = key.clone();
        let send_device_id = device_id.clone();
        tokio::spawn(async move {
            let mut sender = sender;
            let mut outbound_seq: i64 = 0;
            loop {
                let notified = pending.notify.notified();
                if let Some(message) = pending.pop().await {
                    outbound_seq += 1;
                    if send_secure_ws(
                        &mut sender,
                        &send_key,
                        &send_device_id,
                        outbound_seq,
                        &message,
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }
                    if send_control.should_disconnect_after_flush() && pending.is_empty().await {
                        break;
                    }
                    continue;
                }
                if send_control.should_disconnect_after_flush() {
                    break;
                }
                notified.await;
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
                            let next = match resolve_workspace_active_snapshot_subscriptions(
                                &state,
                                workspace_id,
                                message,
                            )
                            .await
                            {
                                Ok(next) => next,
                                Err(_) => {
                                    pending.clear().await;
                                    if queue_reset_required(&pending, &state, workspace_id)
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

                            pending.clear().await;
                            reset_queued = false;
                            send_control.clear_disconnect_after_flush();
                            if queue_snapshot_payload(&pending, &state, workspace_id)
                                .await
                                .is_err()
                            {
                                break;
                            }

                            let mut next_map = HashMap::new();
                            let mut replay_failed = false;
                            for sub in next {
                                let after_seq = sub.after_seq.unwrap_or(0);
                                let replay = replay_session_events_secure(
                                    &state,
                                    workspace_id,
                                    sub.session_id,
                                    after_seq,
                                    |event| pending.push(event),
                                )
                                .await;
                                match replay {
                                    Ok(ReplayOutcome::Replay { last_sent }) => {
                                        next_map.insert(sub.session_id, SessionCursor { last_sent });
                                    }
                                    Ok(ReplayOutcome::ResetRequired) | Err(_) => {
                                        replay_failed = true;
                                        break;
                                    }
                                };
                            }
                            if replay_failed {
                                pending.clear().await;
                                if queue_reset_required(&pending, &state, workspace_id)
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
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if reset_queued {
                            continue;
                        }
                        pending.clear().await;
                        if queue_reset_required(&pending, &state, workspace_id)
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

                    if reset_queued {
                        continue;
                    }

                    if let WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } = &event {
                        let Some(cursor) = subscriptions.get_mut(&delta.session_id) else {
                            continue;
                        };
                        if let Some(ev) = &delta.event {
                            if ev.seq <= cursor.last_sent {
                                continue;
                            }
                            cursor.last_sent = ev.seq;
                        } else if delta.last_event_seq <= cursor.last_sent {
                            continue;
                        } else {
                            cursor.last_sent = delta.last_event_seq;
                        }
                    }

                    if pending
                        .push(WorkspaceActiveSnapshotWsPayload::Event(event))
                        .await
                        .is_err()
                    {
                        if reset_queued {
                            continue;
                        }
                        pending.clear().await;
                        if queue_reset_required(&pending, &state, workspace_id)
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
        Ok(())
    };

    let (send_task, recv_result) = crate::async_util::race_join_handle(send_task, recv_loop).await;

    send_control.set_disconnect_after_flush();
    pending.notify.notify_one();
    if let Some(send_task) = send_task {
        let _ = send_task.await;
    }

    recv_result.unwrap_or(Ok(()))
}

struct SessionCursor {
    last_sent: i64,
}

async fn send_secure_ws<S>(
    sink: &mut S,
    key: &crate::mobile_e2ee::E2eeKey,
    device_id: &str,
    seq: i64,
    payload: &WorkspaceActiveSnapshotWsPayload,
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

pub(super) async fn terminal_stream_ws(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    let terminal_id = TerminalId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let session = state
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

    tasks.spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            match msg {
                WsMessage::Binary(data) => {
                    session.send_input(data);
                }
                WsMessage::Text(text) => {
                    if let Ok(parsed) = serde_json::from_str::<TerminalClientMessage>(&text) {
                        match parsed {
                            TerminalClientMessage::Resize { cols, rows } => {
                                let _ = session.resize(cols, rows);
                            }
                            TerminalClientMessage::Input { data } => {
                                session.send_input(data.into_bytes());
                            }
                        }
                    } else {
                        session.send_input(text.into_bytes());
                    }
                }
                WsMessage::Close(_) => break,
                WsMessage::Ping(_) | WsMessage::Pong(_) => {}
            }
        }
    });

    let _ = tasks.join_next().await;
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

pub(super) async fn web_session_signal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    state
        .web_sessions
        .get(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let manager = state.web_sessions.clone();
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

struct StreamQueueEntry<T> {
    enqueued_at: Instant,
    message: T,
}

struct StreamQueue<T> {
    pending: Mutex<VecDeque<StreamQueueEntry<T>>>,
    notify: Notify,
    limit: usize,
    max_age: Duration,
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

    async fn push(&self, message: T) -> Result<(), ()> {
        let now = Instant::now();
        let mut guard = self.pending.lock().await;
        if guard.len() >= self.limit {
            return Err(());
        }
        if let Some(front) = guard.front() {
            if now.duration_since(front.enqueued_at) >= self.max_age {
                return Err(());
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

    async fn pop(&self) -> Option<T> {
        let mut guard = self.pending.lock().await;
        guard.pop_front().map(|entry| entry.message)
    }

    async fn is_empty(&self) -> bool {
        self.pending.lock().await.is_empty()
    }
}

struct StreamSendControl {
    disconnect_after_flush: AtomicBool,
}

impl StreamSendControl {
    fn new() -> Self {
        Self {
            disconnect_after_flush: AtomicBool::new(false),
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
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum WorkspaceActiveSnapshotWsPayload {
    Event(WorkspaceActiveSnapshotEvent),
    Snapshot(WorkspaceActiveSnapshotStreamMessage),
    ResetRequired(WorkspaceActiveSnapshotStreamMessage),
}

enum ReplayOutcome {
    Replay { last_sent: i64 },
    ResetRequired,
}

async fn queue_reset_required(
    pending: &StreamQueue<WorkspaceActiveSnapshotWsPayload>,
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> Result<(), ()> {
    let (snapshot_rev, _) = super::load_workspace_active_snapshot_state(state, workspace_id).await;
    if crate::fault_injection::maybe_fail("ctx_http.send_workspace_active_reset").is_err() {
        return Err(());
    }
    pending
        .push(WorkspaceActiveSnapshotWsPayload::ResetRequired(
            WorkspaceActiveSnapshotStreamMessage::ResetRequired {
                latest_rev: snapshot_rev,
            },
        ))
        .await
}

async fn queue_snapshot_payload(
    pending: &StreamQueue<WorkspaceActiveSnapshotWsPayload>,
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> Result<(), ()> {
    state
        .ensure_workspace_active_snapshot_hydrated(workspace_id)
        .await;
    let active_snapshot = state
        .workspace_active_snapshot
        .active_snapshot(workspace_id, i64::MAX)
        .await;
    let active_heads = state
        .workspace_active_snapshot
        .active_heads(workspace_id)
        .await;
    pending
        .push(WorkspaceActiveSnapshotWsPayload::Snapshot(
            WorkspaceActiveSnapshotStreamMessage::Snapshot {
                rev: active_snapshot.snapshot_rev,
                active_snapshot,
                active_heads: Some(active_heads),
            },
        ))
        .await
}

async fn replay_session_events_active<F, Fut>(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    after_seq: i64,
    mut emit: F,
) -> Result<ReplayOutcome, ()>
where
    F: FnMut(WorkspaceActiveSnapshotWsPayload) -> Fut,
    Fut: std::future::Future<Output = Result<(), ()>>,
{
    let (snapshot_rev, _) = super::load_workspace_active_snapshot_state(state, workspace_id).await;
    if crate::fault_injection::maybe_fail("ctx_http.replay_session_events_active.list").is_err() {
        return Ok(ReplayOutcome::ResetRequired);
    }
    let replay = state
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
                emit(WorkspaceActiveSnapshotWsPayload::Event(wrapped)).await?;
            }
            Ok(ReplayOutcome::Replay { last_sent })
        }
        SessionReplayResult::Gap {
            last_known_seq,
            reason,
        } => {
            let gap = WorkspaceActiveSnapshotEvent::SessionGap {
                workspace_id,
                snapshot_rev,
                session_id,
                after_seq,
                reason,
            };
            emit(WorkspaceActiveSnapshotWsPayload::Event(gap)).await?;
            Ok(ReplayOutcome::Replay {
                last_sent: last_known_seq.max(after_seq),
            })
        }
        SessionReplayResult::ResetRequired => Ok(ReplayOutcome::ResetRequired),
    }
}

async fn resolve_workspace_active_snapshot_subscriptions(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    message: WorkspaceActiveSnapshotClientMessage,
) -> Result<Vec<WorkspaceActiveSnapshotSessionSubscription>, ()> {
    match message {
        WorkspaceActiveSnapshotClientMessage::Subscribe {
            session_ids,
            sessions,
            task_ids,
            scope,
            ..
        } => {
            let use_task_scope = scope.is_some() || !task_ids.is_empty();
            if use_task_scope {
                let mut resolved = HashSet::new();
                if matches!(scope, Some(WorkspaceActiveSnapshotSubscribeScope::Active)) {
                    let snapshot = state
                        .workspace_active_snapshot
                        .active_snapshot(workspace_id, i64::MAX)
                        .await;
                    for task in snapshot.active.tasks {
                        let session_id = task
                            .task
                            .primary_session_id
                            .unwrap_or(task.primary_session.session.id);
                        resolved.insert(session_id);
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
                        }
                    }
                }
                let mut after_map: HashMap<SessionId, Option<i64>> = HashMap::new();
                for sub in sessions {
                    after_map.insert(sub.session_id, sub.after_seq);
                }
                let mut next: Vec<WorkspaceActiveSnapshotSessionSubscription> = resolved
                    .into_iter()
                    .map(|session_id| WorkspaceActiveSnapshotSessionSubscription {
                        session_id,
                        after_seq: after_map.get(&session_id).copied().flatten(),
                    })
                    .collect();
                for sub in next.iter_mut() {
                    if sub.after_seq.is_none() {
                        let last_seq = state
                            .workspace_active_snapshot
                            .session_last_event_seq(workspace_id, sub.session_id)
                            .await;
                        sub.after_seq = Some(last_seq);
                    }
                }
                next.sort_by(|a, b| a.session_id.0.cmp(&b.session_id.0));
                return Ok(next);
            }

            let mut next: Vec<WorkspaceActiveSnapshotSessionSubscription> = if !sessions.is_empty()
            {
                sessions
            } else {
                session_ids
                    .into_iter()
                    .map(|session_id| WorkspaceActiveSnapshotSessionSubscription {
                        session_id,
                        after_seq: None,
                    })
                    .collect()
            };
            for sub in next.iter_mut() {
                if sub.after_seq.is_none() {
                    let last_seq = state
                        .workspace_active_snapshot
                        .session_last_event_seq(workspace_id, sub.session_id)
                        .await;
                    sub.after_seq = Some(last_seq);
                }
            }
            next.sort_by(|a, b| a.session_id.0.cmp(&b.session_id.0));
            Ok(next)
        }
    }
}

async fn handle_workspace_active_snapshot_ws(
    socket: WebSocket,
    state: Arc<AppState>,
    workspace_id: WorkspaceId,
) {
    let (sender, mut receiver) = socket.split();
    let pending = Arc::new(StreamQueue::new(
        WORKSPACE_STREAM_QUEUE_LIMIT,
        WORKSPACE_STREAM_QUEUE_MAX_AGE,
    ));
    let send_control = Arc::new(StreamSendControl::new());
    let mut rx = state
        .workspace_active_snapshot
        .subscribe(workspace_id)
        .await;
    let mut subscriptions: HashMap<SessionId, SessionCursor> = HashMap::new();
    let mut reset_queued = false;

    let (snapshot_rev, archived_rev) =
        super::load_workspace_active_snapshot_state(&state, workspace_id).await;
    let ready = WorkspaceActiveSnapshotEvent::Ready {
        workspace_id,
        snapshot_rev,
        archived_rev,
    };
    if pending
        .push(WorkspaceActiveSnapshotWsPayload::Event(ready))
        .await
        .is_err()
    {
        return;
    }

    // Bootstrap a snapshot + subscriptions so clients that don't (or can't) send
    // a subscribe message still receive active heads and deltas.
    state
        .ensure_workspace_active_snapshot_hydrated(workspace_id)
        .await;
    let bootstrap_heads = state.workspace_active_snapshot.active_heads(workspace_id).await;
    for head in bootstrap_heads.heads {
        subscriptions.insert(
            head.session.id,
            SessionCursor {
                last_sent: head.last_event_seq,
            },
        );
    }
    let _ = queue_snapshot_payload(&pending, &state, workspace_id).await;

    let send_task = {
        let pending = pending.clone();
        let send_control = send_control.clone();
        tokio::spawn(async move {
            let mut sender = sender;
            loop {
                let notified = pending.notify.notified();
                if let Some(message) = pending.pop().await {
                    let Ok(text) = serde_json::to_string(&message) else {
                        break;
                    };
                    if sender.send(WsMessage::Text(text)).await.is_err() {
                        break;
                    }
                    if send_control.should_disconnect_after_flush() && pending.is_empty().await {
                        break;
                    }
                    continue;
                }
                if send_control.should_disconnect_after_flush() {
                    break;
                }
                notified.await;
            }
        })
    };
    let recv_loop = async {
        loop {
            tokio::select! {
                msg = receiver.next() => {
                    match msg {
                        Some(Ok(WsMessage::Text(text))) => {
                            if let Ok(message) = serde_json::from_str::<WorkspaceActiveSnapshotClientMessage>(&text) {
                                let next = match resolve_workspace_active_snapshot_subscriptions(
                                    &state,
                                    workspace_id,
                                    message,
                                )
                                .await
                                {
                                    Ok(next) => next,
                                    Err(_) => {
                                        pending.clear().await;
                                        if queue_reset_required(&pending, &state, workspace_id)
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

                                pending.clear().await;
                                reset_queued = false;
                                send_control.clear_disconnect_after_flush();
                                if queue_snapshot_payload(&pending, &state, workspace_id)
                                    .await
                                    .is_err()
                                {
                                    break;
                                }

                                let mut next_map = HashMap::new();
                                let mut replay_failed = false;
                                for sub in next {
                                    let after_seq = sub.after_seq.unwrap_or(0);
                                    let replay = replay_session_events_active(
                                        &state,
                                        workspace_id,
                                        sub.session_id,
                                        after_seq,
                                        |event| pending.push(event),
                                    )
                                    .await;
                                    match replay {
                                        Ok(ReplayOutcome::Replay { last_sent }) => {
                                            next_map.insert(sub.session_id, SessionCursor { last_sent });
                                        }
                                        Ok(ReplayOutcome::ResetRequired) | Err(_) => {
                                            replay_failed = true;
                                            break;
                                        }
                                    };
                                }
                                if replay_failed {
                                    pending.clear().await;
                                    if queue_reset_required(&pending, &state, workspace_id)
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
                            }
                        }
                        Some(Ok(WsMessage::Binary(bytes))) => {
                            if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                                if let Ok(message) = serde_json::from_str::<WorkspaceActiveSnapshotClientMessage>(&text) {
                                    let next = match resolve_workspace_active_snapshot_subscriptions(
                                        &state,
                                        workspace_id,
                                        message,
                                    )
                                    .await
                                    {
                                        Ok(next) => next,
                                        Err(_) => {
                                            pending.clear().await;
                                            if queue_reset_required(&pending, &state, workspace_id)
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

                                    pending.clear().await;
                                    reset_queued = false;
                                    send_control.clear_disconnect_after_flush();
                                    if queue_snapshot_payload(&pending, &state, workspace_id)
                                        .await
                                        .is_err()
                                    {
                                        break;
                                    }

                                    let mut next_map = HashMap::new();
                                    let mut replay_failed = false;
                                    for sub in next {
                                        let after_seq = sub.after_seq.unwrap_or(0);
                                        let replay = replay_session_events_active(
                                            &state,
                                            workspace_id,
                                            sub.session_id,
                                            after_seq,
                                            |event| pending.push(event),
                                        )
                                        .await;
                                        match replay {
                                            Ok(ReplayOutcome::Replay { last_sent }) => {
                                                next_map.insert(sub.session_id, SessionCursor { last_sent });
                                            }
                                            Ok(ReplayOutcome::ResetRequired) | Err(_) => {
                                                replay_failed = true;
                                                break;
                                            }
                                        };
                                    }
                                    if replay_failed {
                                        pending.clear().await;
                                        if queue_reset_required(&pending, &state, workspace_id)
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
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            if reset_queued {
                                continue;
                            }
                            pending.clear().await;
                            if queue_reset_required(&pending, &state, workspace_id)
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

                    if reset_queued {
                        continue;
                    }

                    if let WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } = &event {
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

                    if pending
                        .push(WorkspaceActiveSnapshotWsPayload::Event(event))
                        .await
                        .is_err()
                    {
                        if reset_queued {
                            continue;
                        }
                        pending.clear().await;
                        if queue_reset_required(&pending, &state, workspace_id)
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
    };

    let (send_task, _recv_result) = crate::async_util::race_join_handle(send_task, recv_loop).await;

    send_control.set_disconnect_after_flush();
    pending.notify.notify_one();
    if let Some(send_task) = send_task {
        let _ = send_task.await;
    }
}

async fn replay_session_events_secure<F, Fut>(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    after_seq: i64,
    mut emit: F,
) -> Result<ReplayOutcome, ()>
where
    F: FnMut(WorkspaceActiveSnapshotWsPayload) -> Fut,
    Fut: std::future::Future<Output = Result<(), ()>>,
{
    let (snapshot_rev, _) = super::load_workspace_active_snapshot_state(state, workspace_id).await;
    if crate::fault_injection::maybe_fail("ctx_http.replay_session_events_secure.list").is_err() {
        return Ok(ReplayOutcome::ResetRequired);
    }
    let replay = state
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
                emit(WorkspaceActiveSnapshotWsPayload::Event(wrapped)).await?;
            }
            Ok(ReplayOutcome::Replay { last_sent })
        }
        SessionReplayResult::Gap {
            last_known_seq,
            reason,
        } => {
            let gap = WorkspaceActiveSnapshotEvent::SessionGap {
                workspace_id,
                snapshot_rev,
                session_id,
                after_seq,
                reason,
            };
            emit(WorkspaceActiveSnapshotWsPayload::Event(gap)).await?;
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
