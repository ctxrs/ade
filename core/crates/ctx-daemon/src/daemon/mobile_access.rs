use std::sync::Arc;

use chrono::{DateTime, Utc};
use ctx_core::ids::{ConnectionProfileId, MobileDeviceId, WorkspaceId};
use ctx_core::models::{MobileConnectionProfile, MobileDeviceRegistration};
use ctx_store::store::{MobileAccessConfig, MobileDeviceSeqAdvance, MobileDeviceUpsert};
use ctx_transport_runtime::{
    mobile_e2ee::{self, E2eeKey},
    mobile_tunnel::{MobileTunnelState, StartMobileTunnelConfig},
};
use serde::{Deserialize, Serialize};

use crate::daemon::{CoreHandle, DaemonState};

mod control_plane;
mod lifecycle;
mod pairing;
mod profiles;
mod secure_envelope;
mod secure_proxy;
mod tokens;

pub use lifecycle::{
    mobile_public_url_is_allowed, EnableMobileAccessResult, MobileAccessRouteError,
    MobileAccessRouteErrorKind,
};
pub use profiles::{
    CreateMobileConnectionProfileForRouteRequest, CreateMobileConnectionProfileForRouteResult,
    MobileConnectionProfileRouteParams, RegisterMobileDeviceForRouteRequest,
};
pub use secure_envelope::{
    MobileSecureEnvelopeForRoute, MobileSecureProxyPayload, MobileSecureProxyResponsePayload,
    MobileSecureResponseEncryption, OpenMobileSecureRequestResult,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MobileScope {
    DeviceRegistration,
    WorkspaceRead,
    WorkspaceStream,
}

impl MobileScope {
    fn bit(self) -> u8 {
        match self {
            Self::DeviceRegistration => 1 << 0,
            Self::WorkspaceRead => 1 << 1,
            Self::WorkspaceStream => 1 << 2,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::DeviceRegistration => "device_registration",
            Self::WorkspaceRead => "workspace_read",
            Self::WorkspaceStream => "workspace_stream",
        }
    }

    pub fn missing_error(self) -> &'static str {
        match self {
            Self::DeviceRegistration => "mobile profile lacks device_registration scope",
            Self::WorkspaceRead => "mobile profile lacks workspace_read scope",
            Self::WorkspaceStream => "mobile profile lacks workspace_stream scope",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MobileScopeSet(u8);

impl MobileScopeSet {
    fn empty() -> Self {
        Self(0)
    }

    fn insert(&mut self, scope: MobileScope) {
        self.0 |= scope.bit();
    }

    pub fn managed_default() -> Self {
        let mut set = Self::empty();
        set.insert(MobileScope::DeviceRegistration);
        set.insert(MobileScope::WorkspaceRead);
        set.insert(MobileScope::WorkspaceStream);
        set
    }

    pub fn allows(self, scope: MobileScope) -> bool {
        self.0 & scope.bit() != 0
    }

    fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn to_strings(self) -> Vec<String> {
        [
            MobileScope::DeviceRegistration,
            MobileScope::WorkspaceRead,
            MobileScope::WorkspaceStream,
        ]
        .into_iter()
        .filter(|scope| self.allows(*scope))
        .map(|scope| scope.as_str().to_string())
        .collect()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MobileAuthContext {
    pub profile_id: ConnectionProfileId,
    scopes: MobileScopeSet,
}

impl MobileAuthContext {
    pub fn allows(self, scope: MobileScope) -> bool {
        self.scopes.allows(scope)
    }
}

#[derive(Debug, Clone)]
pub struct EnableMobileAccessRequest {
    pub supabase_token: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairMobileDeviceRequest {
    pub device_id: String,
    pub public_key: String,
    pub seq: i64,
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairMobileDevicePayload {
    pub pairing_token: String,
    pub device_label: Option<String>,
    pub platform: Option<String>,
    pub app_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MobileSecureEnvelope {
    pub device_id: String,
    pub seq: i64,
    pub nonce: String,
    pub ciphertext: String,
}

impl From<mobile_e2ee::Envelope> for MobileSecureEnvelope {
    fn from(envelope: mobile_e2ee::Envelope) -> Self {
        Self {
            device_id: envelope.device_id,
            seq: envelope.seq,
            nonce: envelope.nonce_b64,
            ciphertext: envelope.ciphertext_b64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileAuthContextError {
    Store,
}

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
    fn into_store_config(self) -> MobileAccessConfig {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileDeviceSequenceAdvance {
    Advanced,
    Stale { current: i64 },
    Missing,
}

impl From<MobileDeviceSeqAdvance> for MobileDeviceSequenceAdvance {
    fn from(outcome: MobileDeviceSeqAdvance) -> Self {
        match outcome {
            MobileDeviceSeqAdvance::Advanced => Self::Advanced,
            MobileDeviceSeqAdvance::Stale { current } => Self::Stale { current },
            MobileDeviceSeqAdvance::Missing => Self::Missing,
        }
    }
}

pub fn default_mobile_profile_scopes() -> Vec<String> {
    MobileScopeSet::managed_default().to_strings()
}

pub fn mobile_scope_set_from_strings(scopes: &[String]) -> Result<MobileScopeSet, String> {
    let mut set = MobileScopeSet::empty();
    for raw_scope in scopes {
        let scope = raw_scope.trim();
        if scope.is_empty() {
            continue;
        }
        let parsed = match scope {
            "device_registration" => MobileScope::DeviceRegistration,
            "workspace_read" => MobileScope::WorkspaceRead,
            "workspace_stream" => MobileScope::WorkspaceStream,
            _ => return Err(format!("unknown mobile scope: {scope}")),
        };
        set.insert(parsed);
    }
    if set.is_empty() {
        return Err("at least one mobile scope is required".to_string());
    }
    Ok(set)
}

fn mobile_auth_context_from_profile(
    profile: &MobileConnectionProfile,
) -> Result<MobileAuthContext, String> {
    let scopes = mobile_scope_set_from_strings(&profile.scopes)?;
    Ok(MobileAuthContext {
        profile_id: profile.id,
        scopes,
    })
}

fn log_invalid_mobile_scope_set(profile_id: ConnectionProfileId, error: &str) {
    tracing::warn!(
        profile_id = %profile_id.0,
        error,
        "rejecting mobile profile with invalid scope configuration"
    );
}

fn mobile_profile_uses_legacy_empty_scope_shape(profile: &MobileConnectionProfile) -> bool {
    profile.scopes.iter().all(|scope| scope.trim().is_empty())
}

async fn migrate_legacy_mobile_profile_scopes(
    state: &Arc<DaemonState>,
    profile: &MobileConnectionProfile,
) -> Result<MobileAuthContext, MobileAuthContextError> {
    let scopes = default_mobile_profile_scopes();
    state
        .global_store()
        .update_mobile_connection_profile_scopes(profile.id, scopes.clone())
        .await
        .map_err(|e| {
            tracing::error!(
                profile_id = %profile.id.0,
                "failed to migrate legacy mobile profile scopes: {e:?}"
            );
            MobileAuthContextError::Store
        })?;
    tracing::info!(
        profile_id = %profile.id.0,
        "migrated legacy mobile profile to explicit default scopes"
    );
    let scopes = mobile_scope_set_from_strings(&scopes).map_err(|error| {
        tracing::error!(
            profile_id = %profile.id.0,
            error,
            "default mobile scope bundle became invalid"
        );
        MobileAuthContextError::Store
    })?;
    Ok(MobileAuthContext {
        profile_id: profile.id,
        scopes,
    })
}

pub async fn resolve_mobile_auth_context(
    state: &Arc<DaemonState>,
    profile: MobileConnectionProfile,
) -> Result<Option<MobileAuthContext>, MobileAuthContextError> {
    match mobile_auth_context_from_profile(&profile) {
        Ok(auth) => Ok(Some(auth)),
        Err(error) => {
            if mobile_profile_uses_legacy_empty_scope_shape(&profile) {
                let auth = migrate_legacy_mobile_profile_scopes(state, &profile).await?;
                Ok(Some(auth))
            } else {
                log_invalid_mobile_scope_set(profile.id, &error);
                Ok(None)
            }
        }
    }
}

pub async fn load_mobile_auth_context_for_profile(
    state: &Arc<DaemonState>,
    profile_id: ConnectionProfileId,
) -> Result<Option<MobileAuthContext>, MobileAuthContextError> {
    let profile = state
        .global_store()
        .get_mobile_connection_profile(profile_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to load mobile connection profile: {e:?}");
            MobileAuthContextError::Store
        })?;
    match profile {
        Some(profile) => resolve_mobile_auth_context(state, profile).await,
        None => Ok(None),
    }
}

pub async fn verify_mobile_api_token_hash(
    state: &Arc<DaemonState>,
    hash: &str,
) -> Result<Option<MobileAuthContext>, MobileAuthContextError> {
    let profile = state
        .global_store()
        .get_mobile_connection_profile_by_token_hash(hash)
        .await
        .map_err(|e| {
            tracing::error!("failed to query mobile connection profile: {e:?}");
            MobileAuthContextError::Store
        })?;
    if let Some(profile) = profile {
        if let Err(err) = state
            .global_store()
            .mark_mobile_connection_profile_used(profile.id)
            .await
        {
            tracing::warn!("failed to update profile usage: {err:?}");
        }
        resolve_mobile_auth_context(state, profile).await
    } else {
        Ok(None)
    }
}

#[derive(Clone)]
pub struct MobileSecureStreamContext {
    pub device_id: String,
    pub key: E2eeKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileSecureStreamAccessError {
    BadDeviceId,
    Unauthorized,
    NotFound,
    Store,
}

pub async fn require_mobile_secure_stream_access(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    device_id: &str,
    provided_token: &str,
) -> Result<(), MobileSecureStreamAccessError> {
    let device_uuid =
        uuid::Uuid::parse_str(device_id).map_err(|_| MobileSecureStreamAccessError::BadDeviceId)?;
    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|_| MobileSecureStreamAccessError::Store)?
        .ok_or(MobileSecureStreamAccessError::Unauthorized)?;
    if !cfg.enabled {
        return Err(MobileSecureStreamAccessError::Unauthorized);
    }
    let Some(mobile_auth) = load_mobile_auth_context_for_profile(state, cfg.profile_id)
        .await
        .map_err(|_| MobileSecureStreamAccessError::Store)?
    else {
        return Err(MobileSecureStreamAccessError::Unauthorized);
    };
    if !mobile_auth.allows(MobileScope::WorkspaceStream) {
        return Err(MobileSecureStreamAccessError::Unauthorized);
    }
    let device = state
        .global_store()
        .get_mobile_device(MobileDeviceId(device_uuid))
        .await
        .map_err(|_| MobileSecureStreamAccessError::Store)?
        .ok_or(MobileSecureStreamAccessError::Unauthorized)?;
    if device.profile_id != cfg.profile_id {
        return Err(MobileSecureStreamAccessError::Unauthorized);
    }
    if device
        .public_key
        .as_deref()
        .is_none_or(|key| key.trim().is_empty())
    {
        return Err(MobileSecureStreamAccessError::Unauthorized);
    }
    let key = mobile_e2ee::derive_key(
        device_id,
        device.public_key.as_deref().unwrap_or_default(),
        &cfg.daemon_private_key,
    )
    .map_err(|_| MobileSecureStreamAccessError::Unauthorized)?;
    let expected_token = mobile_e2ee::derive_stream_token(&key, &workspace_id.0.to_string());
    if provided_token != expected_token {
        return Err(MobileSecureStreamAccessError::Unauthorized);
    }
    let workspace_exists = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| MobileSecureStreamAccessError::Store)?
        .is_some();
    if !workspace_exists {
        return Err(MobileSecureStreamAccessError::NotFound);
    }
    Ok(())
}

impl CoreHandle {
    pub async fn enable_mobile_access_for_route(
        &self,
        request: EnableMobileAccessRequest,
    ) -> Result<EnableMobileAccessResult, MobileAccessRouteError> {
        lifecycle::enable_mobile_access_for_route(&self.state, request).await
    }

    pub async fn disable_mobile_access_for_route(
        &self,
        supabase_token: String,
    ) -> Result<(), DisableMobileAccessError> {
        lifecycle::disable_mobile_access_for_route(&self.state, supabase_token).await
    }

    pub async fn create_mobile_connection_profile_for_route(
        &self,
        request: CreateMobileConnectionProfileForRouteRequest,
    ) -> Result<CreateMobileConnectionProfileForRouteResult, MobileAccessRouteError> {
        profiles::create_mobile_connection_profile_for_route(&self.state, request).await
    }

    pub async fn list_mobile_connection_profiles_for_route(
        &self,
    ) -> Result<Vec<MobileConnectionProfile>, MobileAccessRouteError> {
        profiles::list_mobile_connection_profiles_for_route(&self.state).await
    }

    pub async fn delete_mobile_connection_profile_for_route(
        &self,
        profile_id: ConnectionProfileId,
    ) -> Result<(), MobileAccessRouteError> {
        profiles::delete_mobile_connection_profile_for_route(&self.state, profile_id).await
    }

    pub async fn delete_mobile_connection_profile_for_route_params(
        &self,
        params: MobileConnectionProfileRouteParams,
    ) -> Result<(), MobileAccessRouteError> {
        profiles::delete_mobile_connection_profile_for_route_params(&self.state, params).await
    }

    pub async fn list_mobile_devices_for_profile_for_route(
        &self,
        profile_id: ConnectionProfileId,
    ) -> Result<Vec<MobileDeviceRegistration>, MobileAccessRouteError> {
        profiles::list_mobile_devices_for_profile_for_route(&self.state, profile_id).await
    }

    pub async fn list_mobile_devices_for_profile_for_route_params(
        &self,
        params: MobileConnectionProfileRouteParams,
    ) -> Result<Vec<MobileDeviceRegistration>, MobileAccessRouteError> {
        profiles::list_mobile_devices_for_profile_for_route_params(&self.state, params).await
    }

    pub async fn register_mobile_device_for_route(
        &self,
        auth: MobileAuthContext,
        request: RegisterMobileDeviceForRouteRequest,
    ) -> Result<MobileDeviceRegistration, MobileAccessRouteError> {
        profiles::register_mobile_device_for_route(&self.state, auth, request).await
    }

    pub async fn pair_mobile_device_for_route(
        &self,
        request: PairMobileDeviceRequest,
    ) -> Result<MobileSecureEnvelope, MobileAccessRouteError> {
        pairing::pair_mobile_device_for_route(&self.state, request).await
    }

    pub async fn open_mobile_secure_request_for_route(
        &self,
        request: MobileSecureEnvelopeForRoute,
    ) -> Result<OpenMobileSecureRequestResult, MobileAccessRouteError> {
        secure_envelope::open_mobile_secure_request_for_route(&self.state, request).await
    }

    pub async fn encrypt_mobile_secure_response_for_route(
        &self,
        context: MobileSecureResponseEncryption,
        response: MobileSecureProxyResponsePayload,
    ) -> Result<MobileSecureEnvelope, MobileAccessRouteError> {
        secure_envelope::encrypt_mobile_secure_response_for_route(context, response).await
    }

    pub async fn create_mobile_connection_profile(
        &self,
        label: String,
        base_url: String,
        token_hash: String,
        token_prefix: String,
        scopes: Vec<String>,
    ) -> anyhow::Result<MobileConnectionProfile> {
        self.state
            .global_store()
            .create_mobile_connection_profile(label, base_url, token_hash, token_prefix, scopes)
            .await
    }

    pub async fn list_mobile_connection_profiles(
        &self,
    ) -> anyhow::Result<Vec<MobileConnectionProfile>> {
        self.state
            .global_store()
            .list_mobile_connection_profiles()
            .await
    }

    pub async fn get_mobile_connection_profile(
        &self,
        profile_id: ConnectionProfileId,
    ) -> anyhow::Result<Option<MobileConnectionProfile>> {
        self.state
            .global_store()
            .get_mobile_connection_profile(profile_id)
            .await
    }

    pub async fn update_mobile_connection_profile_scopes(
        &self,
        profile_id: ConnectionProfileId,
        scopes: Vec<String>,
    ) -> anyhow::Result<()> {
        self.state
            .global_store()
            .update_mobile_connection_profile_scopes(profile_id, scopes)
            .await
    }

    pub async fn delete_mobile_connection_profile(
        &self,
        profile_id: ConnectionProfileId,
    ) -> anyhow::Result<()> {
        self.state
            .global_store()
            .delete_mobile_connection_profile(profile_id)
            .await
    }

    pub async fn get_mobile_access_config(
        &self,
    ) -> anyhow::Result<Option<MobileAccessConfigSnapshot>> {
        self.state
            .global_store()
            .get_mobile_access_config()
            .await
            .map(|config| config.map(Into::into))
    }

    pub async fn upsert_mobile_access_config(
        &self,
        config: MobileAccessConfigUpsert,
    ) -> anyhow::Result<MobileAccessConfigSnapshot> {
        self.state
            .global_store()
            .upsert_mobile_access_config(config.into_store_config())
            .await
            .map(Into::into)
    }

    pub async fn insert_mobile_pairing_token(
        &self,
        token_id: &str,
        token_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        self.state
            .global_store()
            .insert_mobile_pairing_token(token_id, token_hash, expires_at)
            .await
    }

    pub async fn consume_mobile_pairing_token(&self, token_hash: &str) -> anyhow::Result<bool> {
        self.state
            .global_store()
            .consume_mobile_pairing_token(token_hash)
            .await
    }

    pub async fn list_mobile_devices(
        &self,
        profile_id: ConnectionProfileId,
    ) -> anyhow::Result<Vec<MobileDeviceRegistration>> {
        self.state
            .global_store()
            .list_mobile_devices(profile_id)
            .await
    }

    pub async fn get_mobile_device(
        &self,
        device_id: MobileDeviceId,
    ) -> anyhow::Result<Option<MobileDeviceRegistration>> {
        self.state.global_store().get_mobile_device(device_id).await
    }

    pub async fn upsert_mobile_device(
        &self,
        device_id: MobileDeviceId,
        profile_id: ConnectionProfileId,
        update: MobileDeviceRegistrationUpdate,
    ) -> anyhow::Result<MobileDeviceRegistration> {
        self.state
            .global_store()
            .upsert_mobile_device(device_id, profile_id, update.into())
            .await
    }

    pub async fn advance_mobile_device_seq(
        &self,
        device_id: MobileDeviceId,
        seq: i64,
    ) -> anyhow::Result<MobileDeviceSequenceAdvance> {
        self.state
            .global_store()
            .advance_mobile_device_seq(device_id, seq)
            .await
            .map(Into::into)
    }

    pub async fn load_mobile_auth_context_for_profile(
        &self,
        profile_id: ConnectionProfileId,
    ) -> Result<Option<MobileAuthContext>, MobileAuthContextError> {
        load_mobile_auth_context_for_profile(&self.state, profile_id).await
    }

    pub async fn verify_mobile_api_token_hash(
        &self,
        hash: &str,
    ) -> Result<Option<MobileAuthContext>, MobileAuthContextError> {
        verify_mobile_api_token_hash(&self.state, hash).await
    }

    pub async fn mobile_access_status(
        &self,
    ) -> Result<MobileAccessStatusSnapshot, MobileAccessStatusError> {
        mobile_access_status(&self.state).await
    }

    pub async fn disable_mobile_access_runtime(&self) -> Result<(), DisableMobileAccessError> {
        disable_mobile_access_runtime(&self.state).await
    }

    pub async fn start_mobile_tunnel_best_effort(&self, request: StartMobileTunnelRequest) {
        start_mobile_tunnel_best_effort(&self.state, request).await;
    }

    pub async fn require_mobile_secure_stream_access(
        &self,
        workspace_id: WorkspaceId,
        device_id: &str,
        token: &str,
    ) -> Result<(), MobileSecureStreamAccessError> {
        require_mobile_secure_stream_access(&self.state, workspace_id, device_id, token).await
    }

    pub async fn load_mobile_secure_stream_context(
        &self,
        device_id: String,
    ) -> Result<MobileSecureStreamContext, anyhow::Error> {
        load_mobile_secure_stream_context(&self.state, device_id).await
    }
}

pub async fn load_mobile_secure_stream_context(
    state: &Arc<DaemonState>,
    device_id: String,
) -> Result<MobileSecureStreamContext, anyhow::Error> {
    let device_uuid = uuid::Uuid::parse_str(&device_id)?;
    let config = state.global_store().get_mobile_access_config().await?;
    let config = config.ok_or_else(|| anyhow::anyhow!("mobile access not configured"))?;
    if !config.enabled {
        return Err(anyhow::anyhow!("mobile access not enabled"));
    }

    let Some(mobile_auth) = load_mobile_auth_context_for_profile(state, config.profile_id)
        .await
        .map_err(|_| anyhow::anyhow!("failed to load mobile access profile"))?
    else {
        return Err(anyhow::anyhow!(
            "{}",
            MobileScope::WorkspaceStream.missing_error()
        ));
    };
    if !mobile_auth.allows(MobileScope::WorkspaceStream) {
        return Err(anyhow::anyhow!(
            "{}",
            MobileScope::WorkspaceStream.missing_error()
        ));
    }

    let device = state
        .global_store()
        .get_mobile_device(MobileDeviceId(device_uuid))
        .await?
        .ok_or_else(|| anyhow::anyhow!("device not registered"))?;
    if device.profile_id != config.profile_id {
        return Err(anyhow::anyhow!("device not authorized for tunnel"));
    }
    let device_public_key = device
        .public_key
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("device missing public key"))?;
    let key = mobile_e2ee::derive_key(&device_id, device_public_key, &config.daemon_private_key)?;

    Ok(MobileSecureStreamContext { device_id, key })
}

#[derive(Debug, Clone)]
pub struct MobileAccessStatusSnapshot {
    pub enabled: bool,
    pub tunnel_id: Option<String>,
    pub public_base_url: Option<String>,
    pub relay_base_url: Option<String>,
    pub daemon_public_key: Option<String>,
    pub tunnel_state: MobileTunnelState,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartMobileTunnelRequest {
    pub relay_base_url: String,
    pub tunnel_id: String,
    pub tunnel_secret: String,
    pub public_base_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileAccessStatusError {
    ReadConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisableMobileAccessError {
    ReadConfig,
    ClearPairingTokens,
    DeleteConfig,
    DeleteConnectionProfile,
}

pub async fn mobile_access_status(
    state: &Arc<DaemonState>,
) -> Result<MobileAccessStatusSnapshot, MobileAccessStatusError> {
    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|err| {
            tracing::error!("failed to read mobile access config: {err:?}");
            MobileAccessStatusError::ReadConfig
        })?;
    let tunnel_status = state.transport.mobile_tunnel.status().await;
    let (enabled, tunnel_id, public_base_url, relay_base_url, daemon_public_key) = match cfg {
        Some(cfg) => (
            cfg.enabled,
            Some(cfg.tunnel_id),
            Some(cfg.public_base_url),
            Some(cfg.relay_base_url),
            Some(cfg.daemon_public_key),
        ),
        None => (false, None, None, None, None),
    };
    Ok(MobileAccessStatusSnapshot {
        enabled,
        tunnel_id,
        public_base_url,
        relay_base_url,
        daemon_public_key,
        tunnel_state: tunnel_status.state,
        last_error: tunnel_status.last_error,
    })
}

pub async fn start_mobile_tunnel_best_effort(
    state: &Arc<DaemonState>,
    request: StartMobileTunnelRequest,
) {
    let tunnel_cfg = StartMobileTunnelConfig {
        relay_base_url: request.relay_base_url,
        tunnel_id: request.tunnel_id,
        tunnel_secret: request.tunnel_secret,
        public_base_url: request.public_base_url.trim_end_matches('/').to_string(),
        local_daemon_url: state.core.daemon_url.trim_end_matches('/').to_string(),
    };
    if let Err(err) = state.transport.mobile_tunnel.start(tunnel_cfg).await {
        tracing::warn!("failed to start mobile tunnel: {err:#}");
    }
}

pub async fn disable_mobile_access_runtime(
    state: &Arc<DaemonState>,
) -> Result<(), DisableMobileAccessError> {
    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|err| {
            tracing::error!(
                "failed to read mobile access config while disabling mobile access: {err:?}"
            );
            DisableMobileAccessError::ReadConfig
        })?;

    state.transport.mobile_tunnel.stop().await;
    state
        .global_store()
        .clear_mobile_pairing_tokens()
        .await
        .map_err(|err| {
            tracing::error!(
                "failed to clear pairing tokens while disabling mobile access: {err:?}"
            );
            DisableMobileAccessError::ClearPairingTokens
        })?;

    if let Some(cfg) = cfg {
        delete_mobile_access_config(state, cfg.profile_id).await?;
    }
    Ok(())
}

async fn delete_mobile_access_config(
    state: &Arc<DaemonState>,
    profile_id: ConnectionProfileId,
) -> Result<(), DisableMobileAccessError> {
    state
        .global_store()
        .delete_mobile_access_config()
        .await
        .map_err(|err| {
            tracing::error!(
                "failed to delete mobile access config while disabling mobile access: {err:?}"
            );
            DisableMobileAccessError::DeleteConfig
        })?;
    state
        .global_store()
        .delete_mobile_connection_profile(profile_id)
        .await
        .map_err(|err| {
            tracing::error!(
                "failed to delete mobile connection profile while disabling mobile access: {err:?}"
            );
            DisableMobileAccessError::DeleteConnectionProfile
        })?;
    Ok(())
}
