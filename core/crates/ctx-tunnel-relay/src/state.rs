use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::Message;
use ctx_tunnel_store::TunnelStore;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};

#[derive(Clone)]
pub(crate) struct RelayState {
    pub(crate) tunnels: Arc<RwLock<HashMap<String, Arc<Tunnel>>>>,
    pub(crate) master_secret: Vec<u8>,
    pub(crate) store: TunnelStore,
    pub(crate) relay_id: String,
}

impl RelayState {
    pub(crate) fn new(
        master_secret: Vec<u8>,
        store: TunnelStore,
        relay_id: String,
        tunnels: Arc<RwLock<HashMap<String, Arc<Tunnel>>>>,
    ) -> Self {
        Self {
            tunnels,
            master_secret,
            store,
            relay_id,
        }
    }
}

pub(crate) struct Tunnel {
    pub(crate) inner: Mutex<TunnelInner>,
}

impl Tunnel {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(TunnelInner::new()),
        }
    }
}

pub(crate) struct TunnelInner {
    pub(crate) desktop: Option<DesktopHandle>,
    pub(crate) pending_http: HashMap<String, oneshot::Sender<HttpResponse>>,
    pub(crate) pending_ws_open: HashMap<String, oneshot::Sender<Result<(), String>>>,
    pub(crate) ws_streams: HashMap<String, WsStreamHandle>,
}

impl TunnelInner {
    pub(crate) fn new() -> Self {
        Self {
            desktop: None,
            pending_http: HashMap::new(),
            pending_ws_open: HashMap::new(),
            ws_streams: HashMap::new(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct DesktopHandle {
    pub(crate) tx: mpsc::UnboundedSender<RelayToClient>,
}

#[derive(Clone)]
pub(crate) struct WsStreamHandle {
    pub(crate) mobile_tx: mpsc::UnboundedSender<Message>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum RelayToClient {
    HttpRequest {
        id: String,
        method: String,
        path: String,
        headers: Vec<(String, String)>,
        body_b64: String,
    },
    WsOpen {
        id: String,
        path: String,
        headers: Vec<(String, String)>,
    },
    WsMessage {
        id: String,
        is_binary: bool,
        data: String,
    },
    WsClose {
        id: String,
        code: Option<u16>,
        reason: Option<String>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ClientToRelay {
    HttpResponse {
        id: String,
        status: u16,
        headers: Vec<(String, String)>,
        body_b64: String,
    },
    WsOpenResult {
        id: String,
        ok: bool,
        #[serde(default)]
        error: Option<String>,
    },
    WsMessage {
        id: String,
        is_binary: bool,
        data: String,
    },
    WsClosed {
        id: String,
        #[serde(default)]
        code: Option<u16>,
        #[serde(default)]
        reason: Option<String>,
    },
}

#[derive(Debug)]
pub(crate) struct HttpResponse {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

pub(crate) async fn get_or_create_tunnel(state: &RelayState, tunnel_id: &str) -> Arc<Tunnel> {
    {
        let map = state.tunnels.read().await;
        if let Some(tunnel) = map.get(tunnel_id) {
            return tunnel.clone();
        }
    }

    let mut map = state.tunnels.write().await;
    map.entry(tunnel_id.to_string())
        .or_insert_with(|| Arc::new(Tunnel::new()))
        .clone()
}

pub(crate) static BASE64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;
