use std::sync::Arc;

use axum::http::StatusCode;

use crate::daemon::AppState;

use super::super::{
    default_mobile_profile_scopes, mobile_scope_set_from_strings, MobileScope, MobileScopeSet,
};

pub(in crate::api) use self::context::{load_mobile_auth_context_for_profile, MobileAuthContext};
pub(in crate::api) use self::tokens::{
    generate_mobile_api_token, generate_pairing_token, hash_api_token, hash_pairing_token,
};

use context::resolve_mobile_auth_context;

#[path = "mobile/context.rs"]
mod context;
#[path = "mobile/tokens.rs"]
mod tokens;

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
