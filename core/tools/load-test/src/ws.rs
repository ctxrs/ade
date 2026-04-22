use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::task::JoinHandle;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use url::Url;

use ctx_client::Client;
use ctx_core::ids::{SessionId, WorkspaceId};
use ctx_core::models::{
    SessionEventType, WorkspaceActiveSnapshotClientMessage, WorkspaceActiveSnapshotSessionReplay,
    WorkspaceActiveSnapshotSessionSubscription,
};

use crate::metrics::{Metrics, PendingState};
use crate::output::{EventRecord, EventWriter};

const WS_REPLAY_READ_TIMEOUT_MS: u64 = 500;

pub(crate) async fn spawn_ws_listener(
    client: &Client,
    auth_token: Option<&str>,
    workspace_id: WorkspaceId,
    sessions: Vec<SessionId>,
    metrics: Arc<Metrics>,
    pending: Arc<Mutex<PendingState>>,
    mut events: EventWriter,
) -> Result<Option<JoinHandle<()>>> {
    if sessions.is_empty() {
        return Ok(None);
    }
    let mut url =
        Url::parse(&client.workspace_stream_url(workspace_id)?).context("parsing ws url")?;
    if let Some(token) = auth_token {
        url.query_pairs_mut().append_pair("token", token);
    }

    let (ws_stream, _) = connect_async(url.to_string())
        .await
        .context("connecting ws")?;
    let (mut write, mut read) = ws_stream.split();

    let message = WorkspaceActiveSnapshotClientMessage::Subscribe {
        session_ids: sessions,
        sessions: Vec::new(),
        task_ids: Vec::new(),
        vcs_open_session_ids: Vec::new(),
        foreground_session_id: None,
        scope: None,
        include_active_heads: false,
    };
    let payload = serde_json::to_string(&message)?;
    write.send(WsMessage::Text(payload.into())).await?;

    let handle = tokio::spawn(async move {
        while let Some(Ok(msg)) = read.next().await {
            let text = match msg {
                WsMessage::Text(text) => text.to_string(),
                WsMessage::Binary(bytes) => String::from_utf8(bytes.to_vec()).unwrap_or_default(),
                _ => continue,
            };
            let message: ctx_core::models::WorkspaceActiveSnapshotStreamMessage =
                match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
            let mut deltas = Vec::new();
            match message {
                ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Event { event, .. } => {
                    if let ctx_core::models::WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                        delta,
                        ..
                    } = event.as_ref()
                    {
                        deltas.push((**delta).clone());
                    }
                }
                ctx_core::models::WorkspaceActiveSnapshotStreamMessage::HeadsBatch {
                    deltas: batch,
                    ..
                } => {
                    deltas.extend(batch);
                }
                _ => {}
            }
            for delta in deltas {
                if let Some(ev) = delta.event.as_ref() {
                    let content = extract_event_content(&ev.event_type, &ev.payload_json);
                    if let Some(content) = content {
                        if matches!(&ev.event_type, SessionEventType::AssistantChunk) {
                            let input = content.strip_prefix("echo: ").unwrap_or(&content);
                            let mut guard = pending.lock().unwrap();
                            if guard.pending.contains_key(input)
                                && !guard.first_chunked.contains(input)
                            {
                                guard.first_chunked.insert(input.to_string());
                                if let Some(start) = guard.pending.get(input) {
                                    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                                    metrics.first_chunk_ms.lock().unwrap().push(elapsed);
                                    events
                                        .write(EventRecord {
                                            ts_ms: chrono::Utc::now().timestamp_millis() as u128,
                                            kind: "first_chunk",
                                            value_ms: Some(elapsed),
                                            detail: Some(input.to_string()),
                                        })
                                        .ok();
                                }
                            }
                        }
                        if matches!(&ev.event_type, SessionEventType::AssistantComplete) {
                            let input = content.strip_prefix("done: ").unwrap_or(&content);
                            let mut guard = pending.lock().unwrap();
                            if let Some(start) = guard.pending.remove(input) {
                                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                                metrics.done_ms.lock().unwrap().push(elapsed);
                                *metrics.done.lock().unwrap() += 1;
                                events
                                    .write(EventRecord {
                                        ts_ms: chrono::Utc::now().timestamp_millis() as u128,
                                        kind: "message_done",
                                        value_ms: Some(elapsed),
                                        detail: Some(input.to_string()),
                                    })
                                    .ok();
                            }
                        }
                    }
                }
            }
        }
    });

    Ok(Some(handle))
}

pub(crate) async fn run_ws_replay_once(
    client: &Client,
    auth_token: Option<&str>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
) -> Result<()> {
    let mut url =
        Url::parse(&client.workspace_stream_url(workspace_id)?).context("parsing ws url")?;
    if let Some(token) = auth_token {
        url.query_pairs_mut().append_pair("token", token);
    }
    let (ws_stream, _) = connect_async(url.to_string())
        .await
        .context("connecting ws")?;
    let (mut write, mut read) = ws_stream.split();

    let message = WorkspaceActiveSnapshotClientMessage::Subscribe {
        session_ids: Vec::new(),
        sessions: vec![WorkspaceActiveSnapshotSessionSubscription {
            session_id,
            replay: WorkspaceActiveSnapshotSessionReplay::Resume {
                after_seq: 0,
                after_projection_rev: 0,
            },
        }],
        task_ids: Vec::new(),
        vcs_open_session_ids: Vec::new(),
        foreground_session_id: None,
        scope: None,
        include_active_heads: false,
    };
    let payload = serde_json::to_string(&message)?;
    write.send(WsMessage::Text(payload.into())).await?;

    let deadline = Instant::now() + Duration::from_millis(WS_REPLAY_READ_TIMEOUT_MS);
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let wait = remaining.min(Duration::from_millis(200));
        match tokio::time::timeout(wait, read.next()).await {
            Ok(Some(Ok(_))) => break,
            Ok(Some(Err(err))) => return Err(anyhow::anyhow!(err)),
            Ok(None) => break,
            Err(_) => {}
        }
    }

    Ok(())
}

fn extract_event_content(event_type: &SessionEventType, payload: &Value) -> Option<String> {
    match event_type {
        SessionEventType::AssistantChunk | SessionEventType::AssistantComplete => payload
            .get("content")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                payload
                    .get("content_fragment")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            }),
        _ => None,
    }
}
