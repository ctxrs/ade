use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use sha2::Digest;
use tokio::task::JoinHandle;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use url::Url;

use ctx_client::Client;
use ctx_core::ids::{SessionId, WorkspaceId};
use ctx_core::models::{
    SessionEventType, WorkspaceActiveSnapshotClientMessage, WorkspaceActiveSnapshotSessionReplay,
    WorkspaceActiveSnapshotSessionIntent, WorkspaceActiveSnapshotSessionSubscription,
};

use crate::metrics::{Metrics, PendingState};
use crate::output::{EventRecord, EventWriter};

const WS_REPLAY_READ_TIMEOUT_MS: u64 = 500;

fn workspace_active_snapshot_query_token(auth_token: &str, workspace_id: WorkspaceId) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"ctx-browser-stream|");
    hasher.update(format!("workspace_active_snapshot:{}", workspace_id.0).as_bytes());
    hasher.update(b"|");
    hasher.update(auth_token.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn append_workspace_stream_query_token(
    url: &mut Url,
    auth_token: Option<&str>,
    workspace_id: WorkspaceId,
) {
    let Some(auth_token) = auth_token else {
        return;
    };
    url.query_pairs_mut().append_pair(
        "token",
        &workspace_active_snapshot_query_token(auth_token, workspace_id),
    );
}

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
    append_workspace_stream_query_token(&mut url, auth_token, workspace_id);

    let (ws_stream, _) = connect_async(url.to_string())
        .await
        .context("connecting ws")?;
    let (mut write, mut read) = ws_stream.split();

    let message = WorkspaceActiveSnapshotClientMessage::Subscribe {
        session_ids: sessions,
        sessions: Vec::new(),
        task_ids: Vec::new(),
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
    append_workspace_stream_query_token(&mut url, auth_token, workspace_id);
    let (ws_stream, _) = connect_async(url.to_string())
        .await
        .context("connecting ws")?;
    let (mut write, mut read) = ws_stream.split();

    let message = WorkspaceActiveSnapshotClientMessage::Subscribe {
        session_ids: Vec::new(),
        sessions: vec![WorkspaceActiveSnapshotSessionSubscription {
            session_id,
            intent: Some(WorkspaceActiveSnapshotSessionIntent::Replay),
            replay: WorkspaceActiveSnapshotSessionReplay::Resume {
                after_seq: 0,
                after_projection_rev: 0,
            },
        }],
        task_ids: Vec::new(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_workspace_stream_query_token_uses_scoped_token() {
        let workspace_id =
            WorkspaceId(uuid::Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap());
        let mut url =
            Url::parse("wss://example.com/api/workspaces/11111111-1111-1111-1111-111111111111/active_snapshot/stream")
                .unwrap();
        append_workspace_stream_query_token(&mut url, Some("daemon-secret"), workspace_id);
        let token = url
            .query_pairs()
            .find_map(|(key, value)| (key == "token").then_some(value.into_owned()))
            .unwrap();
        assert_eq!(token.len(), 64);
        assert_ne!(token, "daemon-secret");
        assert_eq!(
            token,
            workspace_active_snapshot_query_token("daemon-secret", workspace_id)
        );
    }

    #[test]
    fn workspace_stream_query_token_varies_by_workspace() {
        let auth_token = "daemon-secret";
        let workspace_a =
            WorkspaceId(uuid::Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap());
        let workspace_b =
            WorkspaceId(uuid::Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap());
        assert_ne!(
            workspace_active_snapshot_query_token(auth_token, workspace_a),
            workspace_active_snapshot_query_token(auth_token, workspace_b)
        );
    }
}
