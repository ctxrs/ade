use std::sync::Arc;

use axum::extract::ws::{CloseFrame, Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{protocol::CloseFrame as TungsteniteCloseFrame, Message as TungsteniteMessage},
};

use super::super::web_sessions::{require_web_session_stream_access, WebSessionStreamAccessQuery};
use crate::daemon::AppState;
use crate::web_sessions::WebSessionManager;

pub(crate) async fn web_session_signal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<WebSessionStreamAccessQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    require_web_session_stream_access(&state.transport.web_sessions, &id, query.token.as_deref())
        .await?;
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
    let url = format!("ws://127.0.0.1:{port}/signal");
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
                    let frame = frame.map(|frame| TungsteniteCloseFrame {
                        code: frame.code.into(),
                        reason: frame.reason.to_string().into(),
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
                    let frame = frame.map(|frame| CloseFrame {
                        code: frame.code.into(),
                        reason: frame.reason.to_string().into(),
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
