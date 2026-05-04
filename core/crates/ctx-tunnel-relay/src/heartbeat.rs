use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use ctx_tunnel_store::{RelayAssignmentValidation, RelayHeartbeat, TunnelStore};
use tokio::sync::RwLock;
use tracing::warn;

use crate::state::{RelayState, Tunnel};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

pub(crate) async fn health(State(state): State<RelayState>) -> Response {
    match state.store.health_snapshot().await {
        Ok(snapshot) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "relay_count": snapshot.relay_count,
                "healthy_relay_count": snapshot.healthy_relay_count,
            })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "ok": false,
                "error": err.to_string(),
            })),
        )
            .into_response(),
    }
}

pub(crate) async fn validate_tunnel_for_relay(
    state: &RelayState,
    tunnel_id: &str,
) -> std::result::Result<(), StatusCode> {
    match state
        .store
        .validate_tunnel_relay_assignment(tunnel_id, &state.relay_id)
        .await
    {
        Ok(RelayAssignmentValidation::Valid) => Ok(()),
        Ok(
            RelayAssignmentValidation::UnknownTunnel
            | RelayAssignmentValidation::TunnelDisabled
            | RelayAssignmentValidation::WrongRelay { .. },
        ) => Err(StatusCode::NOT_FOUND),
        Err(err) => {
            warn!("failed to validate relay assignment: {err}");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

pub(crate) async fn relay_heartbeat_loop(
    store: TunnelStore,
    relay_id: String,
    tunnels: Arc<RwLock<HashMap<String, Arc<Tunnel>>>>,
) {
    let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
    loop {
        interval.tick().await;
        if let Err(err) = send_relay_heartbeat(&store, &relay_id, &tunnels).await {
            warn!("failed to record relay heartbeat: {err}");
        }
    }
}

pub(crate) async fn send_relay_heartbeat(
    store: &TunnelStore,
    relay_id: &str,
    tunnels: &Arc<RwLock<HashMap<String, Arc<Tunnel>>>>,
) -> Result<()> {
    let count = active_connected_tunnel_count(tunnels).await;
    store
        .heartbeat_relay(RelayHeartbeat {
            relay_id: relay_id.to_string(),
            active_tunnel_count: count,
            observed_at: Utc::now(),
        })
        .await
        .context("recording relay heartbeat")
}

pub(crate) async fn active_connected_tunnel_count(
    tunnels: &Arc<RwLock<HashMap<String, Arc<Tunnel>>>>,
) -> i32 {
    let snapshot = {
        let map = tunnels.read().await;
        map.values().cloned().collect::<Vec<_>>()
    };

    let mut count: i32 = 0;
    for tunnel in snapshot {
        if tunnel.inner.lock().await.desktop.is_some() && count < i32::MAX {
            count += 1;
        }
    }
    count
}
