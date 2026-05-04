use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use axum::extract::ws::{Message, WebSocket};
use axum::http::HeaderMap;
use base64::Engine;
use futures::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::heartbeat::validate_tunnel_for_relay;
use crate::http_proxy::extract_forward_headers;
use crate::state::{get_or_create_tunnel, RelayState, RelayToClient, WsStreamHandle, BASE64};

pub(crate) async fn handle_mobile_ws(
    state: RelayState,
    tunnel_id: String,
    path: String,
    query: Option<String>,
    headers: HeaderMap,
    mut socket: WebSocket,
) -> Result<()> {
    validate_tunnel_for_relay(&state, &tunnel_id)
        .await
        .map_err(|status| anyhow!("relay rejected tunnel assignment with status {status}"))?;
    let tunnel = get_or_create_tunnel(&state, &tunnel_id).await;

    let desktop = {
        let inner = tunnel.inner.lock().await;
        inner.desktop.clone()
    }
    .context("tunnel not connected")?;

    let stream_id = Uuid::new_v4().to_string();
    let (open_tx, open_rx) = oneshot::channel::<Result<(), String>>();
    {
        let mut inner = tunnel.inner.lock().await;
        inner.pending_ws_open.insert(stream_id.clone(), open_tx);
    }

    let forward_headers = extract_forward_headers(&headers);
    let mut full_path = format!("/{}", path.trim_start_matches('/'));
    if let Some(query) = query {
        if !query.is_empty() {
            full_path.push('?');
            full_path.push_str(&query);
        }
    }

    desktop
        .tx
        .send(RelayToClient::WsOpen {
            id: stream_id.clone(),
            path: full_path,
            headers: forward_headers,
        })
        .context("sending ws open")?;

    match tokio::time::timeout(Duration::from_secs(15), open_rx).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(error))) => {
            let _ = socket
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: axum::extract::ws::close_code::ERROR,
                    reason: error.into(),
                })))
                .await;
            return Ok(());
        }
        _ => {
            let _ = socket.send(Message::Close(None)).await;
            return Ok(());
        }
    }

    let (mobile_tx, mut mobile_rx) = mpsc::unbounded_channel::<Message>();
    {
        let mut inner = tunnel.inner.lock().await;
        inner
            .ws_streams
            .insert(stream_id.clone(), WsStreamHandle { mobile_tx });
    }

    let (mut ws_tx, mut ws_rx) = socket.split();
    let send_task = tokio::spawn(async move {
        while let Some(msg) = mobile_rx.recv().await {
            if ws_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Text(text) => {
                let _ = desktop.tx.send(RelayToClient::WsMessage {
                    id: stream_id.clone(),
                    is_binary: false,
                    data: text,
                });
            }
            Message::Binary(bytes) => {
                let _ = desktop.tx.send(RelayToClient::WsMessage {
                    id: stream_id.clone(),
                    is_binary: true,
                    data: BASE64.encode(bytes),
                });
            }
            Message::Close(frame) => {
                let (code, reason) = frame
                    .map(|frame| (Some(frame.code), Some(frame.reason.to_string())))
                    .unwrap_or((None, None));
                let _ = desktop.tx.send(RelayToClient::WsClose {
                    id: stream_id.clone(),
                    code,
                    reason,
                });
                break;
            }
            _ => {}
        }
    }

    send_task.abort();
    {
        let mut inner = tunnel.inner.lock().await;
        inner.ws_streams.remove(&stream_id);
        inner.pending_ws_open.remove(&stream_id);
    }

    let _ = desktop.tx.send(RelayToClient::WsClose {
        id: stream_id,
        code: None,
        reason: None,
    });

    Ok(())
}
