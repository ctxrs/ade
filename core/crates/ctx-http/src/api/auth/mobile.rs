use std::sync::Arc;

use axum::http::StatusCode;

use crate::daemon::{
    mobile_access::{self as daemon_mobile_access, MobileAuthContext},
    AppState,
};

pub(in crate::api) use self::tokens::{
    generate_mobile_api_token, generate_pairing_token, hash_api_token, hash_pairing_token,
};

#[path = "mobile/tokens.rs"]
mod tokens;

pub(super) async fn verify_mobile_api_token(
    state: &Arc<AppState>,
    token: &str,
) -> Result<Option<MobileAuthContext>, StatusCode> {
    let hash = hash_api_token(token);
    daemon_mobile_access::verify_mobile_api_token_hash(state, &hash)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}
