use anyhow::{anyhow, Context, Result};
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub(crate) const TUNNEL_SECRET_HEADER: &str = "x-ctx-tunnel-secret";

pub(crate) struct RelayConfig {
    pub(crate) database_url: String,
    pub(crate) relay_id: String,
    pub(crate) region: String,
    pub(crate) public_base_url: String,
    pub(crate) internal_base_url: String,
    pub(crate) max_active_tunnels: i32,
}

pub(crate) fn load_master_secret() -> Result<Vec<u8>> {
    let raw = std::env::var("CTX_TUNNEL_MASTER_SECRET")
        .context("CTX_TUNNEL_MASTER_SECRET must be set")?;
    parse_master_secret(&raw)
}

pub(crate) fn load_relay_config() -> Result<RelayConfig> {
    let max_active_tunnels = std::env::var("CTX_TUNNEL_RELAY_MAX_ACTIVE_TUNNELS")
        .context("CTX_TUNNEL_RELAY_MAX_ACTIVE_TUNNELS must be set")?
        .parse::<i32>()
        .context("CTX_TUNNEL_RELAY_MAX_ACTIVE_TUNNELS must be an integer")?;
    if max_active_tunnels <= 0 {
        anyhow::bail!("CTX_TUNNEL_RELAY_MAX_ACTIVE_TUNNELS must be positive");
    }

    Ok(RelayConfig {
        database_url: std::env::var("MOBILE_TUNNEL_DATABASE_URL")
            .context("MOBILE_TUNNEL_DATABASE_URL must be set")?,
        relay_id: std::env::var("CTX_TUNNEL_RELAY_ID")
            .context("CTX_TUNNEL_RELAY_ID must be set")?,
        region: std::env::var("CTX_TUNNEL_RELAY_REGION")
            .context("CTX_TUNNEL_RELAY_REGION must be set")?,
        public_base_url: std::env::var("CTX_TUNNEL_RELAY_PUBLIC_BASE_URL")
            .context("CTX_TUNNEL_RELAY_PUBLIC_BASE_URL must be set")?,
        internal_base_url: std::env::var("CTX_TUNNEL_RELAY_INTERNAL_BASE_URL")
            .context("CTX_TUNNEL_RELAY_INTERNAL_BASE_URL must be set")?,
        max_active_tunnels,
    })
}

pub(crate) fn parse_master_secret(raw: &str) -> Result<Vec<u8>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("CTX_TUNNEL_MASTER_SECRET must not be empty"));
    }
    Ok(trimmed.as_bytes().to_vec())
}

pub(crate) fn derive_secret(master_secret: &[u8], tunnel_id: &str) -> Result<String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(master_secret)
        .map_err(|err| anyhow!("failed to initialize hmac for tunnel secret derivation: {err}"))?;
    mac.update(tunnel_id.as_bytes());
    let bytes = mac.finalize().into_bytes();
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}
