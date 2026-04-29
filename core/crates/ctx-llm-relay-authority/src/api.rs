use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use ctx_llm_relay_contract::{
    RelayDelegationClaims, RelayRequestState, RunGrantClaims, RELAY_AUDIENCE,
};
use serde::{Deserialize, Serialize};

use crate::grant_verifier::GrantVerifier;
use crate::store::{AuthorityError, AuthorityStore, StateEvent};

#[derive(Debug, Clone)]
pub struct RelayAuthorityConfig {
    pub audience: String,
    pub bearer_token: Option<String>,
    pub grant_verifier: Option<GrantVerifier>,
}

impl Default for RelayAuthorityConfig {
    fn default() -> Self {
        Self {
            audience: RELAY_AUDIENCE.to_string(),
            bearer_token: None,
            grant_verifier: None,
        }
    }
}

#[derive(Debug, Clone)]
struct AppState<S> {
    store: S,
    bearer_token: Option<String>,
    grant_verifier: Option<GrantVerifier>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiErrorResponse {
    pub code: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReserveRequest {
    pub delegation: RelayDelegationClaims,
    pub grant: RunGrantClaims,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation_jws: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_jws: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReserveResponse {
    pub request_id: String,
    pub reservation_id: String,
    pub state: RelayRequestState,
    pub reserved_cents: u64,
    pub idempotent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderStartedRequest {
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalizeRequest {
    pub request_id: String,
    pub billable_cents: u64,
    #[serde(default)]
    pub usage_unknown: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_cost_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoidRequest {
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RequestSnapshot {
    pub request_id: String,
    pub reservation_id: Option<String>,
    pub billing_subject_id: Option<String>,
    pub ctx_user_id: Option<String>,
    pub ctx_org_id: Option<String>,
    pub route_id: Option<String>,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub state: RelayRequestState,
    pub reserved_cents: u64,
    pub provider_request_id: Option<String>,
    pub updated_at: DateTime<Utc>,
    pub state_events: Vec<StateEvent>,
}

pub fn relay_authority_router<S>(store: S) -> Router
where
    S: AuthorityStore,
{
    relay_authority_router_with_bearer(store, None)
}

pub fn relay_authority_router_with_bearer<S>(store: S, bearer_token: Option<String>) -> Router
where
    S: AuthorityStore,
{
    relay_authority_router_with_config(
        store,
        RelayAuthorityConfig {
            bearer_token,
            ..RelayAuthorityConfig::default()
        },
    )
}

pub fn relay_authority_router_with_config<S>(store: S, config: RelayAuthorityConfig) -> Router
where
    S: AuthorityStore,
{
    let state = AppState {
        store,
        bearer_token: config.bearer_token,
        grant_verifier: config.grant_verifier,
    };
    Router::new()
        .route("/v1/relay/reservations", post(reserve::<S>))
        .route("/v1/relay/provider-started", post(provider_started::<S>))
        .route("/v1/relay/finalize", post(finalize::<S>))
        .route("/v1/relay/void", post(void::<S>))
        .route("/v1/relay/requests/:request_id", get(get_request::<S>))
        .with_state(state)
}

async fn reserve<S>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    Json(req): Json<ReserveRequest>,
) -> Response
where
    S: AuthorityStore,
{
    if let Err(error) = authorize(&headers, state.bearer_token.as_deref()) {
        return error_response(error);
    }
    let req = match state.grant_verifier.as_ref() {
        Some(verifier) => match verifier.verify_reserve_request(req) {
            Ok(req) => req,
            Err(error) => return error_response(error),
        },
        None => req,
    };
    match state.store.reserve(req).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => error_response(error),
    }
}

async fn provider_started<S>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    Json(req): Json<ProviderStartedRequest>,
) -> Response
where
    S: AuthorityStore,
{
    if let Err(error) = authorize(&headers, state.bearer_token.as_deref()) {
        return error_response(error);
    }
    match state.store.provider_started(req).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => error_response(error),
    }
}

async fn finalize<S>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    Json(req): Json<FinalizeRequest>,
) -> Response
where
    S: AuthorityStore,
{
    if let Err(error) = authorize(&headers, state.bearer_token.as_deref()) {
        return error_response(error);
    }
    match state.store.finalize(req).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => error_response(error),
    }
}

async fn void<S>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    Json(req): Json<VoidRequest>,
) -> Response
where
    S: AuthorityStore,
{
    if let Err(error) = authorize(&headers, state.bearer_token.as_deref()) {
        return error_response(error);
    }
    match state.store.void(req).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => error_response(error),
    }
}

async fn get_request<S>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
) -> Response
where
    S: AuthorityStore,
{
    if let Err(error) = authorize(&headers, state.bearer_token.as_deref()) {
        return error_response(error);
    }
    match state.store.get_request(&request_id).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => error_response(error),
    }
}

fn error_response(error: AuthorityError) -> Response {
    (
        error.status_code(),
        Json(ApiErrorResponse {
            code: error.code().to_string(),
            error: error.to_string(),
        }),
    )
        .into_response()
}

fn authorize(headers: &HeaderMap, bearer_token: Option<&str>) -> Result<(), AuthorityError> {
    let Some(expected) = bearer_token else {
        return Ok(());
    };
    let Some(header_value) = headers.get(header::AUTHORIZATION) else {
        return Err(AuthorityError::Unauthorized(
            "missing authorization header".to_string(),
        ));
    };
    let header_value = header_value
        .to_str()
        .map_err(|_| AuthorityError::Unauthorized("invalid authorization header".to_string()))?;
    let Some(actual) = header_value.strip_prefix("Bearer ") else {
        return Err(AuthorityError::Unauthorized(
            "authorization header must use bearer auth".to_string(),
        ));
    };
    if !constant_time_eq(actual.as_bytes(), expected.as_bytes()) {
        return Err(AuthorityError::Unauthorized(
            "invalid authority bearer token".to_string(),
        ));
    }
    Ok(())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for index in 0..left.len() {
        diff |= left[index] ^ right[index];
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_auth_rejects_missing_or_wrong_tokens() {
        let headers = HeaderMap::new();
        assert!(authorize(&headers, Some("secret")).is_err());

        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer nope".parse().unwrap());
        assert!(authorize(&headers, Some("secret")).is_err());
    }

    #[test]
    fn bearer_auth_accepts_expected_token_and_can_be_disabled() {
        let headers = HeaderMap::new();
        assert!(authorize(&headers, None).is_ok());

        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer secret".parse().unwrap());
        assert!(authorize(&headers, Some("secret")).is_ok());
    }
}
