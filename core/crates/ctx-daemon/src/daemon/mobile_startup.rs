use ctx_store::Store;
use ctx_transport_runtime::mobile_tunnel::MobileTunnelManager;

#[derive(Clone)]
pub(in crate::daemon) struct SavedMobileTunnelReconnectHost {
    auth_token_present: bool,
    global_store: Store,
    local_daemon_url: String,
    mobile_tunnel: MobileTunnelManager,
}

impl SavedMobileTunnelReconnectHost {
    pub(in crate::daemon) fn new(
        auth_token_present: bool,
        global_store: Store,
        local_daemon_url: String,
        mobile_tunnel: MobileTunnelManager,
    ) -> Self {
        Self {
            auth_token_present,
            global_store,
            local_daemon_url,
            mobile_tunnel,
        }
    }
}

pub(super) fn spawn_saved_mobile_tunnel_reconnect(host: SavedMobileTunnelReconnectHost) {
    tokio::spawn(async move {
        if !host.auth_token_present {
            return;
        }
        let cfg = match host.global_store.get_mobile_access_config().await {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!("failed to read saved mobile access config: {err:#}");
                return;
            }
        };
        let Some(cfg) = cfg else {
            return;
        };
        if !cfg.enabled {
            return;
        }

        let start_cfg = ctx_transport_runtime::mobile_tunnel::StartMobileTunnelConfig {
            relay_base_url: cfg.relay_base_url,
            tunnel_id: cfg.tunnel_id,
            tunnel_secret: cfg.tunnel_secret,
            public_base_url: cfg.public_base_url.trim_end_matches('/').to_string(),
            local_daemon_url: host.local_daemon_url.trim_end_matches('/').to_string(),
        };
        if let Err(err) = host.mobile_tunnel.start(start_cfg).await {
            tracing::warn!("failed to start saved mobile tunnel: {err:#}");
        }
    });
}
