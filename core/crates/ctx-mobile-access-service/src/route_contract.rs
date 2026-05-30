use chrono::{DateTime, Utc};
use ctx_core::ids::ConnectionProfileId;
use ctx_core::models::MobileConnectionProfile;
use ctx_transport_runtime::mobile_tunnel::MobileTunnelState;
use serde_json::json;

use crate::{
    CreateMobileConnectionProfileRequest, CreateMobileConnectionProfileResult,
    MobileAccessDisablePersistedStateError, MobileAccessDisableRemainingCleanupError,
    MobileAccessServiceError, MobileAccessServiceErrorKind, RegisterMobileDeviceRequest,
};

pub use crate::{
    MobileSecureEnvelope, MobileSecureEnvelopeForRoute, MobileSecureStreamContext,
    MobileSecureWorkspaceStreamRouteParams, PairMobileDeviceRequest,
};

#[derive(Debug, Clone)]
pub struct EnableMobileAccessRequest {
    pub managed_tunnel_grant: String,
}

#[derive(Debug, Clone)]
pub struct EnableMobileAccessResult {
    pub status: MobileAccessStatusSnapshot,
    pub qr_payload: serde_json::Value,
    pub pairing_expires_at: DateTime<Utc>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisableMobileAccessError {
    ReadConfig,
    DisableConfig,
    ClearPairingTokens,
    DeleteConfig,
    DeleteConnectionProfile,
}

impl From<MobileAccessDisablePersistedStateError> for DisableMobileAccessError {
    fn from(error: MobileAccessDisablePersistedStateError) -> Self {
        match error {
            MobileAccessDisablePersistedStateError::ReadConfig => Self::ReadConfig,
            MobileAccessDisablePersistedStateError::DisableConfig => Self::DisableConfig,
        }
    }
}

impl From<MobileAccessDisableRemainingCleanupError> for DisableMobileAccessError {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileAccessRouteErrorKind {
    BadRequest,
    Unauthorized,
    Forbidden,
    Conflict,
    NotFound,
    BadGateway,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileAccessRouteError {
    kind: MobileAccessRouteErrorKind,
    message: String,
}

impl MobileAccessRouteError {
    pub fn new(kind: MobileAccessRouteErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(MobileAccessRouteErrorKind::BadRequest, message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(MobileAccessRouteErrorKind::Unauthorized, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(MobileAccessRouteErrorKind::Internal, message)
    }

    pub fn kind(&self) -> MobileAccessRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<MobileAccessServiceError> for MobileAccessRouteError {
    fn from(error: MobileAccessServiceError) -> Self {
        let kind = match error.kind() {
            MobileAccessServiceErrorKind::BadRequest => MobileAccessRouteErrorKind::BadRequest,
            MobileAccessServiceErrorKind::Unauthorized => MobileAccessRouteErrorKind::Unauthorized,
            MobileAccessServiceErrorKind::Conflict => MobileAccessRouteErrorKind::Conflict,
            MobileAccessServiceErrorKind::NotFound => MobileAccessRouteErrorKind::NotFound,
            MobileAccessServiceErrorKind::Internal => MobileAccessRouteErrorKind::Internal,
        };
        Self::new(kind, error.message())
    }
}

#[derive(Debug, Clone)]
pub struct CreateMobileConnectionProfileForRouteRequest {
    pub label: String,
    pub base_url: String,
    pub scopes: Vec<String>,
}

impl From<CreateMobileConnectionProfileForRouteRequest> for CreateMobileConnectionProfileRequest {
    fn from(request: CreateMobileConnectionProfileForRouteRequest) -> Self {
        Self {
            label: request.label,
            base_url: request.base_url,
            scopes: request.scopes,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CreateMobileConnectionProfileForRouteResult {
    pub profile: MobileConnectionProfile,
    pub token: String,
    pub qr_payload: serde_json::Value,
}

impl From<CreateMobileConnectionProfileResult> for CreateMobileConnectionProfileForRouteResult {
    fn from(result: CreateMobileConnectionProfileResult) -> Self {
        let qr_payload = build_connection_profile_qr_payload(
            &result.profile,
            &result.profile.base_url,
            &result.token,
        );
        Self {
            profile: result.profile,
            token: result.token,
            qr_payload,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MobileConnectionProfileRouteParams {
    profile_id: String,
}

impl MobileConnectionProfileRouteParams {
    pub fn new(profile_id: impl Into<String>) -> Self {
        Self {
            profile_id: profile_id.into(),
        }
    }

    pub fn into_profile_id(self) -> Result<ConnectionProfileId, MobileAccessRouteError> {
        uuid::Uuid::parse_str(&self.profile_id)
            .map(ConnectionProfileId)
            .map_err(|_| {
                MobileAccessRouteError::bad_request("connection profile id must be a UUID")
            })
    }
}

#[derive(Debug, Clone, Default)]
pub struct RegisterMobileDeviceForRouteRequest {
    pub device_id: String,
    pub device_label: Option<String>,
    pub platform: Option<String>,
    pub push_token: Option<String>,
    pub push_provider: Option<String>,
    pub public_key: Option<String>,
    pub app_version: Option<String>,
}

impl From<RegisterMobileDeviceForRouteRequest> for RegisterMobileDeviceRequest {
    fn from(request: RegisterMobileDeviceForRouteRequest) -> Self {
        Self {
            device_id: request.device_id,
            device_label: request.device_label,
            platform: request.platform,
            push_token: request.push_token,
            push_provider: request.push_provider,
            public_key: request.public_key,
            app_version: request.app_version,
        }
    }
}

fn build_connection_profile_qr_payload(
    profile: &MobileConnectionProfile,
    normalized_base: &str,
    token: &str,
) -> serde_json::Value {
    json!({
        "connection_profile": {
            "label": profile.label,
            "connection": {
                "type": "direct_https",
                "base_url": normalized_base,
            },
            "auth": {
                "api_token": token,
            }
        },
        "label": profile.label,
        "baseUrl": normalized_base,
        "token": token,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_route_params_reject_invalid_uuid() {
        let error = MobileConnectionProfileRouteParams::new("not-a-uuid")
            .into_profile_id()
            .unwrap_err();
        assert_eq!(error.kind(), MobileAccessRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "connection profile id must be a UUID");
    }

    #[test]
    fn service_error_mapping_preserves_route_categories_and_message() {
        for (service_error, expected_kind) in [
            (
                MobileAccessServiceError::bad_request("message"),
                MobileAccessRouteErrorKind::BadRequest,
            ),
            (
                MobileAccessServiceError::unauthorized("message"),
                MobileAccessRouteErrorKind::Unauthorized,
            ),
            (
                MobileAccessServiceError::conflict("message"),
                MobileAccessRouteErrorKind::Conflict,
            ),
            (
                MobileAccessServiceError::not_found("message"),
                MobileAccessRouteErrorKind::NotFound,
            ),
            (
                MobileAccessServiceError::internal("message"),
                MobileAccessRouteErrorKind::Internal,
            ),
        ] {
            let route_error = MobileAccessRouteError::from(service_error);
            assert_eq!(route_error.kind(), expected_kind);
            assert_eq!(route_error.message(), "message");
        }
    }

    #[test]
    fn profile_creation_result_preserves_qr_payload_shape() {
        let profile = MobileConnectionProfile {
            id: ConnectionProfileId::new(),
            label: "phone".to_string(),
            base_url: "https://ctx.example".to_string(),
            token_prefix: "tok".to_string(),
            scopes: vec!["workspace.read".to_string()],
            created_at: Utc::now(),
            last_used_at: None,
        };
        let result = CreateMobileConnectionProfileForRouteResult::from(
            CreateMobileConnectionProfileResult {
                profile,
                token: "token-value".to_string(),
            },
        );

        assert_eq!(result.qr_payload["label"], "phone");
        assert_eq!(result.qr_payload["baseUrl"], "https://ctx.example");
        assert_eq!(result.qr_payload["token"], "token-value");
        assert_eq!(
            result.qr_payload["connection_profile"]["connection"]["type"],
            "direct_https"
        );
    }
}
