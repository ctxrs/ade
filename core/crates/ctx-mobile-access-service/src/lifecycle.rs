use chrono::{DateTime, Duration, Utc};
use ctx_core::ids::ConnectionProfileId;
use ctx_store::Store;

use crate::{
    default_mobile_profile_scopes, generate_mobile_api_token, generate_pairing_token,
    hash_api_token, hash_pairing_token, MobileAccessConfigSnapshot, MobileAccessConfigUpsert,
    MobileAccessServiceError,
};

#[derive(Debug, Clone)]
pub struct PersistMobileAccessEnableBootstrapRequest {
    pub public_base_url: String,
    pub relay_base_url: String,
    pub tunnel_id: String,
    pub tunnel_secret: String,
    pub daemon_public_key: String,
    pub daemon_private_key: String,
    pub now: DateTime<Utc>,
    pub pairing_token_ttl_seconds: i64,
}

#[derive(Debug, Clone)]
pub struct PersistMobileAccessEnableBootstrapResult {
    pub config: MobileAccessConfigSnapshot,
    pub pairing_token: String,
    pub pairing_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileAccessDisableCleanupError {
    ReadConfig,
    DisableConfig,
    ClearPairingTokens,
    DeleteConfig,
    DeleteConnectionProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileAccessDisablePersistedStateError {
    ReadConfig,
    DisableConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileAccessDisableRemainingCleanupError {
    ClearPairingTokens,
    DeleteConfig,
    DeleteConnectionProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistedMobileAccessDisable {
    profile_id: Option<ConnectionProfileId>,
}

impl PersistedMobileAccessDisable {
    pub fn profile_id(self) -> Option<ConnectionProfileId> {
        self.profile_id
    }
}

struct ManagedMobileAccessKeys {
    daemon_public_key: String,
    daemon_private_key: String,
    profile_id: ConnectionProfileId,
    created_at: DateTime<Utc>,
}

pub async fn persist_mobile_access_enable_bootstrap(
    store: &Store,
    request: PersistMobileAccessEnableBootstrapRequest,
) -> Result<PersistMobileAccessEnableBootstrapResult, MobileAccessServiceError> {
    let public_base_url = request.public_base_url.trim_end_matches('/').to_string();
    let keys = load_or_create_managed_mobile_access_keys(store, &request, &public_base_url).await?;
    let config = persist_mobile_access_config(store, &request, &public_base_url, &keys).await?;
    let (pairing_token, pairing_expires_at) =
        create_mobile_pairing_bootstrap(store, request.now, request.pairing_token_ttl_seconds)
            .await?;
    Ok(PersistMobileAccessEnableBootstrapResult {
        config,
        pairing_token,
        pairing_expires_at,
    })
}

pub async fn persist_mobile_access_disable_cleanup(
    store: &Store,
) -> Result<(), MobileAccessDisableCleanupError> {
    let disabled = persist_mobile_access_disabled_state(store)
        .await
        .map_err(MobileAccessDisableCleanupError::from)?;
    finish_mobile_access_disable_cleanup(store, disabled)
        .await
        .map_err(MobileAccessDisableCleanupError::from)
}

pub async fn persist_mobile_access_disabled_state(
    store: &Store,
) -> Result<PersistedMobileAccessDisable, MobileAccessDisablePersistedStateError> {
    let cfg = store.get_mobile_access_config().await.map_err(|err| {
        tracing::error!(
            "failed to read mobile access config while disabling mobile access: {err:?}"
        );
        MobileAccessDisablePersistedStateError::ReadConfig
    })?;

    let profile_id = cfg.as_ref().map(|cfg| cfg.profile_id);
    if profile_id.is_some() {
        store
            .set_mobile_access_enabled(false)
            .await
            .map_err(|err| {
                tracing::error!("failed to mark mobile access disabled before cleanup: {err:?}");
                MobileAccessDisablePersistedStateError::DisableConfig
            })?;
    }

    Ok(PersistedMobileAccessDisable { profile_id })
}

pub async fn finish_mobile_access_disable_cleanup(
    store: &Store,
    disabled: PersistedMobileAccessDisable,
) -> Result<(), MobileAccessDisableRemainingCleanupError> {
    store.clear_mobile_pairing_tokens().await.map_err(|err| {
        tracing::error!("failed to clear pairing tokens while disabling mobile access: {err:?}");
        MobileAccessDisableRemainingCleanupError::ClearPairingTokens
    })?;

    if let Some(profile_id) = disabled.profile_id {
        store.delete_mobile_access_config().await.map_err(|err| {
            tracing::error!(
                "failed to delete mobile access config while disabling mobile access: {err:?}"
            );
            MobileAccessDisableRemainingCleanupError::DeleteConfig
        })?;
        store
            .delete_mobile_connection_profile(profile_id)
            .await
            .map_err(|err| {
                tracing::error!(
                    "failed to delete mobile connection profile while disabling mobile access: {err:?}"
                );
                MobileAccessDisableRemainingCleanupError::DeleteConnectionProfile
            })?;
    }

    Ok(())
}

impl From<MobileAccessDisablePersistedStateError> for MobileAccessDisableCleanupError {
    fn from(error: MobileAccessDisablePersistedStateError) -> Self {
        match error {
            MobileAccessDisablePersistedStateError::ReadConfig => Self::ReadConfig,
            MobileAccessDisablePersistedStateError::DisableConfig => Self::DisableConfig,
        }
    }
}

impl From<MobileAccessDisableRemainingCleanupError> for MobileAccessDisableCleanupError {
    fn from(error: MobileAccessDisableRemainingCleanupError) -> Self {
        match error {
            MobileAccessDisableRemainingCleanupError::ClearPairingTokens => {
                Self::ClearPairingTokens
            }
            MobileAccessDisableRemainingCleanupError::DeleteConfig => Self::DeleteConfig,
            MobileAccessDisableRemainingCleanupError::DeleteConnectionProfile => {
                Self::DeleteConnectionProfile
            }
        }
    }
}

async fn load_or_create_managed_mobile_access_keys(
    store: &Store,
    request: &PersistMobileAccessEnableBootstrapRequest,
    public_base_url: &str,
) -> Result<ManagedMobileAccessKeys, MobileAccessServiceError> {
    match store.get_mobile_access_config().await.map_err(|e| {
        tracing::error!("failed to read mobile access config: {e:?}");
        MobileAccessServiceError::internal("failed to read mobile access config")
    })? {
        Some(cfg) => {
            ensure_managed_profile_scopes(store, cfg.profile_id).await?;
            Ok(ManagedMobileAccessKeys {
                daemon_public_key: cfg.daemon_public_key,
                daemon_private_key: cfg.daemon_private_key,
                profile_id: cfg.profile_id,
                created_at: cfg.created_at,
            })
        }
        None => create_managed_mobile_access_keys(store, request, public_base_url).await,
    }
}

async fn ensure_managed_profile_scopes(
    store: &Store,
    profile_id: ConnectionProfileId,
) -> Result<(), MobileAccessServiceError> {
    let profile = store
        .get_mobile_connection_profile(profile_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to read managed mobile profile: {e:?}");
            MobileAccessServiceError::internal("failed to read managed profile")
        })?
        .ok_or_else(|| MobileAccessServiceError::internal("managed mobile profile is missing"))?;
    if profile.scopes.is_empty() {
        store
            .update_mobile_connection_profile_scopes(profile.id, default_mobile_profile_scopes())
            .await
            .map_err(|e| {
                tracing::error!("failed to backfill managed mobile profile scopes: {e:?}");
                MobileAccessServiceError::internal("failed to update managed profile")
            })?;
    }
    Ok(())
}

async fn create_managed_mobile_access_keys(
    store: &Store,
    request: &PersistMobileAccessEnableBootstrapRequest,
    public_base_url: &str,
) -> Result<ManagedMobileAccessKeys, MobileAccessServiceError> {
    let token = generate_mobile_api_token();
    let token_hash = hash_api_token(&token);
    let token_prefix: String = token.chars().take(8).collect();
    let profile = store
        .create_mobile_connection_profile(
            "Managed Mobile Access".to_string(),
            public_base_url.to_string(),
            token_hash,
            token_prefix,
            default_mobile_profile_scopes(),
        )
        .await
        .map_err(|e| {
            tracing::error!("failed to create managed mobile profile: {e:?}");
            MobileAccessServiceError::internal("failed to create managed profile")
        })?;
    Ok(ManagedMobileAccessKeys {
        daemon_public_key: request.daemon_public_key.clone(),
        daemon_private_key: request.daemon_private_key.clone(),
        profile_id: profile.id,
        created_at: request.now,
    })
}

async fn persist_mobile_access_config(
    store: &Store,
    request: &PersistMobileAccessEnableBootstrapRequest,
    public_base_url: &str,
    keys: &ManagedMobileAccessKeys,
) -> Result<MobileAccessConfigSnapshot, MobileAccessServiceError> {
    let config = MobileAccessConfigUpsert {
        profile_id: keys.profile_id,
        tunnel_id: request.tunnel_id.clone(),
        public_base_url: public_base_url.to_string(),
        relay_base_url: request.relay_base_url.clone(),
        tunnel_secret: request.tunnel_secret.clone(),
        daemon_public_key: keys.daemon_public_key.clone(),
        daemon_private_key: keys.daemon_private_key.clone(),
        enabled: true,
        created_at: keys.created_at,
        updated_at: request.now,
    };

    store
        .upsert_mobile_access_config(config.into_store_config())
        .await
        .map(Into::into)
        .map_err(|e| {
            tracing::error!("failed to persist mobile access config: {e:?}");
            MobileAccessServiceError::internal("failed to persist mobile access config")
        })
}

async fn create_mobile_pairing_bootstrap(
    store: &Store,
    now: DateTime<Utc>,
    ttl_seconds: i64,
) -> Result<(String, DateTime<Utc>), MobileAccessServiceError> {
    let pairing_token = generate_pairing_token();
    let pairing_hash = hash_pairing_token(&pairing_token);
    let expires_at = now + Duration::seconds(ttl_seconds);
    store
        .insert_mobile_pairing_token(&uuid::Uuid::new_v4().to_string(), &pairing_hash, expires_at)
        .await
        .map_err(|e| {
            tracing::error!("failed to persist pairing token: {e:?}");
            MobileAccessServiceError::internal("failed to persist pairing token")
        })?;
    Ok((pairing_token, expires_at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash_pairing_token;

    async fn test_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("mobile-access-lifecycle.sqlite");
        let store = Store::open(&db_path).await.expect("open store");
        (dir, store)
    }

    fn enable_request(now: DateTime<Utc>) -> PersistMobileAccessEnableBootstrapRequest {
        PersistMobileAccessEnableBootstrapRequest {
            public_base_url: "https://mobile.example.test/root/".to_string(),
            relay_base_url: "https://relay.example.test".to_string(),
            tunnel_id: "tunnel-1".to_string(),
            tunnel_secret: "secret-1".to_string(),
            daemon_public_key: "daemon-public".to_string(),
            daemon_private_key: "daemon-private".to_string(),
            now,
            pairing_token_ttl_seconds: 900,
        }
    }

    #[tokio::test]
    async fn enable_bootstrap_creates_profile_config_and_pairing_token() {
        let (_dir, store) = test_store().await;
        let now = Utc::now();
        let result = persist_mobile_access_enable_bootstrap(&store, enable_request(now))
            .await
            .expect("enable bootstrap");

        assert_eq!(
            result.config.public_base_url,
            "https://mobile.example.test/root"
        );
        assert_eq!(result.config.tunnel_id, "tunnel-1");
        assert_eq!(result.config.daemon_public_key, "daemon-public");
        assert!(result.config.enabled);
        assert_eq!(result.pairing_expires_at, now + Duration::seconds(900));
        assert!(store
            .consume_mobile_pairing_token(&hash_pairing_token(&result.pairing_token))
            .await
            .expect("consume pairing token"));
        let profile = store
            .get_mobile_connection_profile(result.config.profile_id)
            .await
            .expect("load profile")
            .expect("profile exists");
        assert_eq!(profile.label, "Managed Mobile Access");
        assert_eq!(profile.scopes, crate::default_mobile_profile_scopes());
    }

    #[tokio::test]
    async fn enable_bootstrap_reuses_existing_managed_keys_and_profile() {
        let (_dir, store) = test_store().await;
        let now = Utc::now();
        let first = persist_mobile_access_enable_bootstrap(&store, enable_request(now))
            .await
            .expect("first bootstrap");
        let mut second_request = enable_request(now + Duration::seconds(1));
        second_request.tunnel_id = "tunnel-2".to_string();
        second_request.daemon_public_key = "new-unused-public".to_string();
        second_request.daemon_private_key = "new-unused-private".to_string();

        let second = persist_mobile_access_enable_bootstrap(&store, second_request)
            .await
            .expect("second bootstrap");

        assert_eq!(second.config.profile_id, first.config.profile_id);
        assert_eq!(
            second.config.daemon_public_key,
            first.config.daemon_public_key
        );
        assert_eq!(
            second.config.daemon_private_key,
            first.config.daemon_private_key
        );
        assert_eq!(second.config.tunnel_id, "tunnel-2");
    }

    #[tokio::test]
    async fn disable_cleanup_removes_restartable_config_tokens_and_profile() {
        let (_dir, store) = test_store().await;
        let result = persist_mobile_access_enable_bootstrap(&store, enable_request(Utc::now()))
            .await
            .expect("enable bootstrap");
        let token_hash = hash_pairing_token(&result.pairing_token);

        persist_mobile_access_disable_cleanup(&store)
            .await
            .expect("disable cleanup");

        assert!(store
            .get_mobile_access_config()
            .await
            .expect("load config")
            .is_none());
        assert!(store
            .get_mobile_connection_profile(result.config.profile_id)
            .await
            .expect("load profile")
            .is_none());
        assert!(!store
            .consume_mobile_pairing_token(&token_hash)
            .await
            .expect("consume pairing token"));
    }
}
