use std::sync::Arc;

use axum::http::StatusCode;
use base64::Engine;
use rand_core::RngCore;
use sha2::Digest;

use crate::daemon::AppState;
use ctx_core::ids::ConnectionProfileId;
use ctx_core::models::MobileConnectionProfile;

use super::super::{
    default_mobile_profile_scopes, mobile_scope_set_from_strings, MobileScope, MobileScopeSet,
};

#[derive(Clone, Copy)]
pub(in crate::api) struct MobileAuthContext {
    pub(in crate::api) profile_id: ConnectionProfileId,
    scopes: MobileScopeSet,
}

impl MobileAuthContext {
    pub(in crate::api) fn allows(self, scope: MobileScope) -> bool {
        self.scopes.allows(scope)
    }
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
    state: &Arc<AppState>,
    profile: &MobileConnectionProfile,
) -> Result<MobileAuthContext, StatusCode> {
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
            StatusCode::INTERNAL_SERVER_ERROR
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
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(MobileAuthContext {
        profile_id: profile.id,
        scopes,
    })
}

async fn resolve_mobile_auth_context(
    state: &Arc<AppState>,
    profile: MobileConnectionProfile,
) -> Result<Option<MobileAuthContext>, StatusCode> {
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

pub(in crate::api) async fn load_mobile_auth_context_for_profile(
    state: &Arc<AppState>,
    profile_id: ConnectionProfileId,
) -> Result<Option<MobileAuthContext>, StatusCode> {
    let profile = state
        .global_store()
        .get_mobile_connection_profile(profile_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to load mobile connection profile: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    match profile {
        Some(profile) => resolve_mobile_auth_context(state, profile).await,
        None => Ok(None),
    }
}

pub(super) async fn verify_mobile_api_token(
    state: &Arc<AppState>,
    token: &str,
) -> Result<Option<MobileAuthContext>, StatusCode> {
    let hash = hash_api_token(token);
    let profile = state
        .global_store()
        .get_mobile_connection_profile_by_token_hash(&hash)
        .await
        .map_err(|e| {
            tracing::error!("failed to query mobile connection profile: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
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

pub(in crate::api) fn hash_api_token(token: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

pub(in crate::api) fn hash_pairing_token(token: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

pub(in crate::api) fn generate_mobile_api_token() -> String {
    format!("ctxm_{}", uuid::Uuid::new_v4().to_string().replace('-', ""))
}

pub(in crate::api) fn generate_pairing_token() -> String {
    let mut bytes = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
