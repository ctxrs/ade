use std::sync::Arc;

use ctx_core::ids::ConnectionProfileId;
use ctx_core::models::{MobileConnectionProfile, MobileDeviceRegistration};
use ctx_mobile_access_service::{
    CreateMobileConnectionProfileRequest, CreateMobileConnectionProfileResult,
    RegisterMobileDeviceRequest,
};
use serde_json::json;

use super::{MobileAccessRouteError, MobileAuthContext};
use crate::daemon::DaemonState;

#[derive(Debug, Clone)]
pub struct CreateMobileConnectionProfileForRouteRequest {
    pub label: String,
    pub base_url: String,
    pub scopes: Vec<String>,
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

pub(super) async fn create_mobile_connection_profile_for_route(
    state: &Arc<DaemonState>,
    request: CreateMobileConnectionProfileForRouteRequest,
) -> Result<CreateMobileConnectionProfileForRouteResult, MobileAccessRouteError> {
    ctx_mobile_access_service::create_mobile_connection_profile(
        state.global_store(),
        CreateMobileConnectionProfileRequest {
            label: request.label,
            base_url: request.base_url,
            scopes: request.scopes,
        },
    )
    .await
    .map(Into::into)
    .map_err(Into::into)
}

pub(super) async fn list_mobile_connection_profiles_for_route(
    state: &Arc<DaemonState>,
) -> Result<Vec<MobileConnectionProfile>, MobileAccessRouteError> {
    ctx_mobile_access_service::list_mobile_connection_profiles(state.global_store())
        .await
        .map_err(Into::into)
}

pub(super) async fn delete_mobile_connection_profile_for_route(
    state: &Arc<DaemonState>,
    profile_id: ConnectionProfileId,
) -> Result<(), MobileAccessRouteError> {
    ctx_mobile_access_service::delete_mobile_connection_profile(state.global_store(), profile_id)
        .await
        .map_err(Into::into)
}

pub(super) async fn delete_mobile_connection_profile_for_route_params(
    state: &Arc<DaemonState>,
    params: MobileConnectionProfileRouteParams,
) -> Result<(), MobileAccessRouteError> {
    let profile_id = parse_connection_profile_route_id(&params.profile_id)?;
    delete_mobile_connection_profile_for_route(state, profile_id).await
}

pub(super) async fn list_mobile_devices_for_profile_for_route(
    state: &Arc<DaemonState>,
    profile_id: ConnectionProfileId,
) -> Result<Vec<MobileDeviceRegistration>, MobileAccessRouteError> {
    ctx_mobile_access_service::list_mobile_devices_for_profile(state.global_store(), profile_id)
        .await
        .map_err(Into::into)
}

pub(super) async fn list_mobile_devices_for_profile_for_route_params(
    state: &Arc<DaemonState>,
    params: MobileConnectionProfileRouteParams,
) -> Result<Vec<MobileDeviceRegistration>, MobileAccessRouteError> {
    let profile_id = parse_connection_profile_route_id(&params.profile_id)?;
    list_mobile_devices_for_profile_for_route(state, profile_id).await
}

pub(super) async fn register_mobile_device_for_route(
    state: &Arc<DaemonState>,
    auth: MobileAuthContext,
    request: RegisterMobileDeviceForRouteRequest,
) -> Result<MobileDeviceRegistration, MobileAccessRouteError> {
    ctx_mobile_access_service::register_mobile_device(state.global_store(), auth, request.into())
        .await
        .map_err(Into::into)
}

fn parse_connection_profile_route_id(
    profile_id: &str,
) -> Result<ConnectionProfileId, MobileAccessRouteError> {
    uuid::Uuid::parse_str(profile_id)
        .map(ConnectionProfileId)
        .map_err(|_| MobileAccessRouteError::bad_request("connection profile id must be a UUID"))
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
    use crate::daemon::mobile_access::MobileAccessRouteErrorKind;

    #[test]
    fn parse_connection_profile_route_id_rejects_invalid_uuid() {
        let error = parse_connection_profile_route_id("not-a-uuid").unwrap_err();
        assert_eq!(error.kind(), MobileAccessRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "connection profile id must be a UUID");
    }
}
