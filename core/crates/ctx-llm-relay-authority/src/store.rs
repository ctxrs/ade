use async_trait::async_trait;
use axum::http::StatusCode;
use ctx_llm_relay_contract::RelayRequestState;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::api::{
    FinalizeRequest, ProviderStartedRequest, RequestSnapshot, ReserveRequest, ReserveResponse,
    VoidRequest,
};

#[derive(Debug, Error)]
pub enum AuthorityError {
    #[error("request rejected: {0}")]
    BadRequest(String),
    #[error("request is unauthorized: {0}")]
    Unauthorized(String),
    #[error("request is forbidden: {0}")]
    Forbidden(String),
    #[error("request conflicts with existing relay state: {0}")]
    Conflict(String),
    #[error("insufficient credits: {0}")]
    InsufficientCredits(String),
    #[error("request not found")]
    NotFound,
    #[error("authority store failed: {0}")]
    Store(String),
}

impl AuthorityError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::InsufficientCredits(_) => StatusCode::PAYMENT_REQUIRED,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::Conflict(_) => "conflict",
            Self::InsufficientCredits(_) => "insufficient_credits",
            Self::NotFound => "not_found",
            Self::Store(_) => "store_error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StateEvent {
    pub request_id: String,
    pub from_state: Option<RelayRequestState>,
    pub to_state: RelayRequestState,
    pub reason: Option<String>,
}

#[async_trait]
pub trait AuthorityStore: Clone + Send + Sync + 'static {
    async fn reserve(&self, request: ReserveRequest) -> Result<ReserveResponse, AuthorityError>;
    async fn provider_started(
        &self,
        request: ProviderStartedRequest,
    ) -> Result<RequestSnapshot, AuthorityError>;
    async fn finalize(&self, request: FinalizeRequest) -> Result<RequestSnapshot, AuthorityError>;
    async fn void(&self, request: VoidRequest) -> Result<RequestSnapshot, AuthorityError>;
    async fn get_request(&self, request_id: &str) -> Result<RequestSnapshot, AuthorityError>;
}
