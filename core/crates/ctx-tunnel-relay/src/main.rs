pub(crate) use std::collections::HashMap;
use std::net::SocketAddr;
pub(crate) use std::sync::Arc;
pub(crate) use std::time::Duration;

use anyhow::{Context, Result};
pub(crate) use axum::body::Bytes;
#[cfg(test)]
pub(crate) use axum::extract::ws::Message;
pub(crate) use axum::extract::{FromRequestParts, Path, State};
pub(crate) use axum::http::{HeaderMap, Method, Request, StatusCode, Uri};
pub(crate) use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
pub(crate) use base64::Engine;
use clap::Parser;
use ctx_tunnel_store::{RelayRegistration, TunnelStore};
#[cfg(test)]
pub(crate) use tokio::sync::{mpsc, Mutex};
pub(crate) use tokio::sync::{oneshot, RwLock};
use tracing::info;
pub(crate) use tracing::warn;
pub(crate) use uuid::Uuid;

mod config;
mod desktop;
mod heartbeat;
mod http_proxy;
mod mobile;
mod state;

#[cfg(test)]
pub(crate) use config::{derive_secret, parse_master_secret, TUNNEL_SECRET_HEADER};
pub(crate) use config::{load_master_secret, load_relay_config, RelayConfig};
pub(crate) use desktop::desktop_connect_ws;
#[cfg(test)]
pub(crate) use desktop::{dispatch_client_message, extract_desktop_secret};
pub(crate) use heartbeat::{
    health, relay_heartbeat_loop, send_relay_heartbeat, validate_tunnel_for_relay,
};
pub(crate) use mobile::handle_mobile_ws;
pub(crate) use state::{get_or_create_tunnel, HttpResponse, RelayState, RelayToClient, BASE64};
#[cfg(test)]
pub(crate) use state::{ClientToRelay, Tunnel, TunnelInner, WsStreamHandle};

#[derive(Parser, Debug)]
#[command(name = "ctx-tunnel-relay", version)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:8787")]
    listen: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let master_secret = load_master_secret()?;
    let config = load_relay_config()?;
    let store = TunnelStore::connect(&config.database_url)
        .await
        .context("connecting to mobile tunnel database")?;
    register_relay(&store, &config).await?;

    let tunnels = Arc::new(RwLock::new(HashMap::new()));
    send_relay_heartbeat(&store, &config.relay_id, &tunnels)
        .await
        .context("recording initial relay heartbeat")?;
    tokio::spawn(relay_heartbeat_loop(
        store.clone(),
        config.relay_id.clone(),
        tunnels.clone(),
    ));

    let state = RelayState::new(master_secret, store, config.relay_id, tunnels);
    let app = build_router(state);

    let addr: SocketAddr = args.listen.parse().context("parsing --listen")?;
    info!("listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("binding listener")?;
    axum::serve(listener, app).await.context("serving relay")?;
    Ok(())
}

async fn register_relay(store: &TunnelStore, config: &RelayConfig) -> Result<()> {
    store
        .register_relay(RelayRegistration {
            relay_id: config.relay_id.clone(),
            region: config.region.clone(),
            public_base_url: config.public_base_url.clone(),
            internal_base_url: config.internal_base_url.clone(),
            max_active_tunnels: config.max_active_tunnels,
        })
        .await
        .context("registering relay node")?;
    Ok(())
}

fn build_router(state: RelayState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/connect/:tunnel_id", get(desktop_connect_ws))
        .route(
            "/t/:tunnel_id/*path",
            get(http_proxy::proxy_get)
                .post(http_proxy::proxy_http)
                .put(http_proxy::proxy_http)
                .delete(http_proxy::proxy_http)
                .patch(http_proxy::proxy_http)
                .options(http_proxy::proxy_http)
                .head(http_proxy::proxy_http),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests;
