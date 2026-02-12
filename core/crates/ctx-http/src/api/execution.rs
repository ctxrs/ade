use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::broadcast;

use ctx_core::ids::WorkspaceId;

use crate::daemon::AppState;
use crate::execution_setup::{
    ExecutionLaunchSnapshot, ExecutionLaunchState, ExecutionLaunchStreamEvent,
};
use crate::logs;
use crate::workspace_config;

use super::errors::ApiErrorResp;

#[derive(Debug, Deserialize)]
pub(super) struct ExecutionLaunchStartReq {
    workspace_id: String,
}

pub(super) async fn launch_start(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExecutionLaunchStartReq>,
) -> Result<Json<ExecutionLaunchSnapshot>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(req.workspace_id.trim()).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid workspace id".to_string(),
                }),
            )
        })?);

    let (workspace, execution_settings) =
        resolve_workspace_execution_settings(&state, workspace_id).await?;

    let snapshot = state
        .execution
        .setup
        .start_workspace_launch(workspace, execution_settings, state.core.daemon_url.clone())
        .await;
    Ok(Json(snapshot))
}

#[derive(Debug, Deserialize)]
pub(super) struct ExecutionLaunchStatusQuery {
    job_id: String,
}

pub(super) async fn launch_status(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ExecutionLaunchStatusQuery>,
) -> Result<Json<ExecutionLaunchSnapshot>, StatusCode> {
    let job_id = query.job_id.trim();
    if job_id.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let snapshot = state
        .execution
        .setup
        .launch_status(job_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(snapshot))
}

pub(super) async fn launch_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Query(query): Query<ExecutionLaunchStatusQuery>,
) -> impl IntoResponse {
    let job_id = query.job_id.trim().to_string();
    if job_id.is_empty() {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let Some((snapshot, rx)) = state.execution.setup.subscribe_launch(&job_id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };

    ws.on_upgrade(move |socket| async move {
        if let Err(err) = handle_launch_stream_ws(socket, snapshot, rx).await {
            tracing::debug!("execution launch stream ended: {err:#}");
        }
    })
    .into_response()
}

async fn handle_launch_stream_ws(
    socket: WebSocket,
    snapshot: ExecutionLaunchSnapshot,
    mut rx: broadcast::Receiver<ExecutionLaunchStreamEvent>,
) -> anyhow::Result<()> {
    let (mut sender, mut receiver) = socket.split();

    send_event(
        &mut sender,
        &ExecutionLaunchStreamEvent::LaunchSnapshot {
            snapshot: snapshot.clone(),
        },
    )
    .await?;

    if !matches!(snapshot.state, ExecutionLaunchState::Running) {
        return Ok(());
    }

    loop {
        tokio::select! {
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(WsMessage::Close(_))) | None => break,
                    Some(Ok(WsMessage::Ping(payload))) => {
                        let _ = sender.send(WsMessage::Pong(payload)).await;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
            event = rx.recv() => {
                match event {
                    Ok(event) => {
                        let is_terminal = matches!(event, ExecutionLaunchStreamEvent::LaunchComplete { .. } | ExecutionLaunchStreamEvent::LaunchError { .. });
                        send_event(&mut sender, &event).await?;
                        if is_terminal {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

async fn send_event<S>(sender: &mut S, event: &ExecutionLaunchStreamEvent) -> anyhow::Result<()>
where
    S: futures::Sink<WsMessage, Error = axum::Error> + Unpin,
{
    let raw = serde_json::to_string(event)?;
    sender.send(WsMessage::Text(raw)).await?;
    Ok(())
}

async fn resolve_workspace_execution_settings(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> Result<
    (
        ctx_core::models::Workspace,
        crate::settings::ExecutionSettings,
    ),
    (StatusCode, Json<ApiErrorResp>),
> {
    let workspace = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let settings = crate::settings::load_settings(state.global_store())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let mut execution_settings = settings.execution.clone().unwrap_or_default();
    let store = state.store_for_workspace(workspace_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    match workspace_config::load_execution_settings_override(&store).await {
        Ok(Some(ov)) => {
            workspace_config::apply_execution_settings_override(&mut execution_settings, &ov)
        }
        Ok(None) => {}
        Err(err) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&err.to_string()),
                }),
            ));
        }
    }

    Ok((workspace, execution_settings))
}
