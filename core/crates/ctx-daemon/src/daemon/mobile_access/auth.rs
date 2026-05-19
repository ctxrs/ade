use std::sync::Arc;

use ctx_core::ids::ConnectionProfileId;
use ctx_core::models::MobileConnectionProfile;

use crate::daemon::DaemonState;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileAuthContextError {
    Store,
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
