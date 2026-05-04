use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use base64::Engine;
use futures::{SinkExt, StreamExt};
use subtle::ConstantTimeEq;
use tokio::sync::mpsc;
use tracing::warn;

use crate::config::{derive_secret, TUNNEL_SECRET_HEADER};
use crate::heartbeat::validate_tunnel_for_relay;
use crate::state::get_or_create_tunnel;
use crate::state::{
    ClientToRelay, DesktopHandle, HttpResponse, RelayState, RelayToClient, Tunnel, BASE64,
};

pub(crate) async fn desktop_connect_ws(
    State(state): State<RelayState>,
    Path(tunnel_id): Path<String>,
    headers: HeaderMap,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> impl IntoResponse {
    if let Err(status) = validate_tunnel_for_relay(&state, &tunnel_id).await {
        return status.into_response();
    }
    let secret = match extract_desktop_secret(&headers) {
        Ok(secret) => secret,
        Err(status) => return status.into_response(),
    };

    ws.on_upgrade(move |socket| async move {
        if let Err(err) = handle_desktop_socket(state, tunnel_id, secret, socket).await {
            warn!("desktop ws ended with error: {err:#}");
        }
    })
    .into_response()
}

pub(crate) fn extract_desktop_secret(
    headers: &HeaderMap,
) -> std::result::Result<String, StatusCode> {
    let Some(value) = headers.get(TUNNEL_SECRET_HEADER) else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    let value = value.to_str().map_err(|_| StatusCode::UNAUTHORIZED)?;
    if value.is_empty() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(value.to_string())
}

pub(crate) async fn handle_desktop_socket(
    state: RelayState,
    tunnel_id: String,
    secret: String,
    socket: WebSocket,
) -> Result<()> {
    let tunnel = get_or_create_tunnel(&state, &tunnel_id).await;

    {
        let mut inner = tunnel.inner.lock().await;
        let expected = match derive_secret(&state.master_secret, &tunnel_id) {
            Ok(value) => value,
            Err(err) => {
                warn!(
                    "rejecting desktop connect for tunnel {tunnel_id}: unable to derive secret: {err:#}"
                );
                return Ok(());
            }
        };
        if secret.as_bytes().ct_eq(expected.as_bytes()).unwrap_u8() != 1 {
            warn!("rejecting desktop connect for tunnel {tunnel_id}: secret mismatch");
            return Ok(());
        }
        clear_desktop_session(&mut inner);
    }

    state
        .store
        .record_desktop_connected(&tunnel_id, &state.relay_id)
        .await
        .context("recording desktop tunnel connection")?;

    let (mut ws_tx, mut ws_rx) = socket.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<RelayToClient>();
    {
        let mut inner = tunnel.inner.lock().await;
        inner.desktop = Some(DesktopHandle { tx: out_tx.clone() });
    }

    let tunnel_for_read = tunnel.clone();
    let tunnel_for_write = tunnel.clone();

    let write_task = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            let text = match serde_json::to_string(&msg) {
                Ok(text) => text,
                Err(err) => {
                    warn!("failed to serialize relay message: {err}");
                    continue;
                }
            };
            if ws_tx.send(Message::Text(text)).await.is_err() {
                break;
            }
        }
    });

    let read_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            match msg {
                Message::Text(text) => match serde_json::from_str::<ClientToRelay>(&text) {
                    Ok(parsed) => dispatch_client_message(&tunnel_for_read, parsed).await,
                    Err(err) => warn!("invalid client message: {err}"),
                },
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = write_task => {},
        _ = read_task => {},
    }

    {
        let mut inner = tunnel_for_write.inner.lock().await;
        clear_desktop_session(&mut inner);
    }
    if let Err(err) = state
        .store
        .record_desktop_disconnected(&tunnel_id, &state.relay_id)
        .await
    {
        warn!("failed to record desktop tunnel disconnect: {err}");
    }

    Ok(())
}

pub(crate) async fn dispatch_client_message(tunnel: &Arc<Tunnel>, msg: ClientToRelay) {
    match msg {
        ClientToRelay::HttpResponse {
            id,
            status,
            headers,
            body_b64,
        } => {
            let body = BASE64.decode(body_b64.as_bytes()).unwrap_or_default();
            let sender = {
                let mut inner = tunnel.inner.lock().await;
                inner.pending_http.remove(&id)
            };
            if let Some(tx) = sender {
                let _ = tx.send(HttpResponse {
                    status,
                    headers,
                    body,
                });
            }
        }
        ClientToRelay::WsOpenResult { id, ok, error } => {
            let sender = {
                let mut inner = tunnel.inner.lock().await;
                inner.pending_ws_open.remove(&id)
            };
            if let Some(tx) = sender {
                let result = if ok {
                    Ok(())
                } else {
                    Err(error.unwrap_or_else(|| "ws open failed".to_string()))
                };
                let _ = tx.send(result);
            }
        }
        ClientToRelay::WsMessage {
            id,
            is_binary,
            data,
        } => {
            let handle = {
                let inner = tunnel.inner.lock().await;
                inner.ws_streams.get(&id).cloned()
            };
            let Some(handle) = handle else {
                return;
            };
            let msg = if is_binary {
                let bytes = BASE64.decode(data.as_bytes()).unwrap_or_default();
                Message::Binary(bytes)
            } else {
                Message::Text(data)
            };
            let _ = handle.mobile_tx.send(msg);
        }
        ClientToRelay::WsClosed { id, code, reason } => {
            let handle = {
                let mut inner = tunnel.inner.lock().await;
                inner.ws_streams.remove(&id)
            };
            let Some(handle) = handle else {
                return;
            };
            let close = axum::extract::ws::CloseFrame {
                code: axum::extract::ws::close_code::NORMAL,
                reason: reason.unwrap_or_default().into(),
            };
            let _ = handle.mobile_tx.send(Message::Close(Some(close)));
            if let Some(_code) = code {
                // ignore for now
            }
        }
    }
}

fn clear_desktop_session(inner: &mut crate::state::TunnelInner) {
    inner.desktop = None;
    inner.pending_http.clear();
    inner.pending_ws_open.clear();
    inner.ws_streams.clear();
}
