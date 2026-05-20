use chrono::{DateTime, Utc};
use ctx_core::ids::ConnectionProfileId;
use ctx_store::store::{MobileAccessConfig, MobileDeviceUpsert};

#[derive(Debug, Clone)]
pub struct MobileAccessConfigSnapshot {
    pub profile_id: ConnectionProfileId,
    pub tunnel_id: String,
    pub public_base_url: String,
    pub relay_base_url: String,
    pub tunnel_secret: String,
    pub daemon_public_key: String,
    pub daemon_private_key: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<MobileAccessConfig> for MobileAccessConfigSnapshot {
    fn from(config: MobileAccessConfig) -> Self {
        Self {
            profile_id: config.profile_id,
            tunnel_id: config.tunnel_id,
            public_base_url: config.public_base_url,
            relay_base_url: config.relay_base_url,
            tunnel_secret: config.tunnel_secret,
            daemon_public_key: config.daemon_public_key,
            daemon_private_key: config.daemon_private_key,
            enabled: config.enabled,
            created_at: config.created_at,
            updated_at: config.updated_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MobileAccessConfigUpsert {
    pub profile_id: ConnectionProfileId,
    pub tunnel_id: String,
    pub public_base_url: String,
    pub relay_base_url: String,
    pub tunnel_secret: String,
    pub daemon_public_key: String,
    pub daemon_private_key: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl MobileAccessConfigUpsert {
    pub fn into_store_config(self) -> MobileAccessConfig {
        MobileAccessConfig {
            id: "default".to_string(),
            profile_id: self.profile_id,
            tunnel_id: self.tunnel_id,
            public_base_url: self.public_base_url,
            relay_base_url: self.relay_base_url,
            tunnel_secret: self.tunnel_secret,
            daemon_public_key: self.daemon_public_key,
            daemon_private_key: self.daemon_private_key,
            enabled: self.enabled,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MobileDeviceRegistrationUpdate {
    pub device_label: Option<String>,
    pub platform: Option<String>,
    pub push_token: Option<String>,
    pub push_provider: Option<String>,
    pub public_key: Option<String>,
    pub app_version: Option<String>,
}

impl From<MobileDeviceRegistrationUpdate> for MobileDeviceUpsert {
    fn from(update: MobileDeviceRegistrationUpdate) -> Self {
        Self {
            device_label: update.device_label,
            platform: update.platform,
            push_token: update.push_token,
            push_provider: update.push_provider,
            public_key: update.public_key,
            app_version: update.app_version,
        }
    }
}
