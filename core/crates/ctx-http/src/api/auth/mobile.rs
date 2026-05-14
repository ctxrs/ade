use axum::http::StatusCode;

use crate::daemon::{mobile_access::MobileAuthContext, CoreHandle};

pub(in crate::api) use self::tokens::{
    generate_mobile_api_token, generate_pairing_token, hash_api_token, hash_pairing_token,
};

#[path = "mobile/tokens.rs"]
mod tokens;

pub(super) async fn verify_mobile_api_token(
    state: &CoreHandle,
    token: &str,
) -> Result<Option<MobileAuthContext>, StatusCode> {
    let hash = hash_api_token(token);
    state
        .verify_mobile_api_token_hash(&hash)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}
