use axum::http::StatusCode;

use ctx_daemon::daemon::{mobile_access::MobileAuthContext, CoreHandle};

#[path = "mobile/tokens.rs"]
mod tokens;
use tokens::hash_api_token;

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
