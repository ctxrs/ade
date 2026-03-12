const DEFAULT_TUNNEL_CONTROL_PLANE_URL: &str = "https://tunnel.ctx.rs";

pub(super) const PAIRING_TOKEN_TTL_SECS: i64 = 10 * 60;

pub(super) fn resolve_control_plane_url() -> String {
    std::env::var("CTX_TUNNEL_CONTROL_PLANE_URL")
        .unwrap_or_else(|_| DEFAULT_TUNNEL_CONTROL_PLANE_URL.to_string())
}
