use std::sync::Arc;

use serde_json::json;
use url::Url;

use ctx_core::ids::{ConnectionProfileId, MobileDeviceId};
use ctx_core::models::{MobileConnectionProfile, MobileDeviceRegistration};

use super::tokens::{generate_mobile_api_token, hash_api_token};
use super::{
    mobile_scope_set_from_strings, MobileAccessRouteError, MobileAccessRouteErrorKind,
    MobileAuthContext, MobileDeviceRegistrationUpdate, MobileScope,
};
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

pub(super) async fn create_mobile_connection_profile_for_route(
    state: &Arc<DaemonState>,
    request: CreateMobileConnectionProfileForRouteRequest,
) -> Result<CreateMobileConnectionProfileForRouteResult, MobileAccessRouteError> {
    let label = request.label.trim();
    if label.is_empty() {
        return Err(MobileAccessRouteError::bad_request("label is required"));
    }
    let normalized_base = normalize_profile_base_url(&request.base_url)?;
    let scopes = mobile_scope_set_from_strings(&request.scopes)
        .map(|scope_set| scope_set.to_strings())
        .map_err(MobileAccessRouteError::bad_request)?;
    let token = generate_mobile_api_token();
    let token_hash = hash_api_token(&token);
    let token_prefix: String = token.chars().take(8).collect();
    let profile = state
        .global_store()
        .create_mobile_connection_profile(
            label.to_string(),
            normalized_base.clone(),
            token_hash,
            token_prefix,
            scopes,
        )
        .await
        .map_err(|e| {
            tracing::error!("failed to create mobile profile: {e:?}");
            MobileAccessRouteError::internal("failed to create profile")
        })?;
    let qr_payload = build_connection_profile_qr_payload(&profile, &normalized_base, &token);
    Ok(CreateMobileConnectionProfileForRouteResult {
        profile,
        token,
        qr_payload,
    })
}

pub(super) async fn list_mobile_connection_profiles_for_route(
    state: &Arc<DaemonState>,
) -> Result<Vec<MobileConnectionProfile>, MobileAccessRouteError> {
    state
        .global_store()
        .list_mobile_connection_profiles()
        .await
        .map_err(|e| {
            tracing::error!("failed to list mobile profiles: {e:?}");
            MobileAccessRouteError::internal("failed to list mobile profiles")
        })
}

pub(super) async fn delete_mobile_connection_profile_for_route(
    state: &Arc<DaemonState>,
    profile_id: ConnectionProfileId,
) -> Result<(), MobileAccessRouteError> {
    if state
        .global_store()
        .get_mobile_connection_profile(profile_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to load mobile profile before delete: {e:?}");
            MobileAccessRouteError::internal("failed to load mobile profile")
        })?
        .is_none()
    {
        return Err(MobileAccessRouteError::new(
            MobileAccessRouteErrorKind::NotFound,
            "mobile profile not found",
        ));
    }
    state
        .global_store()
        .delete_mobile_connection_profile(profile_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to delete mobile profile: {e:?}");
            MobileAccessRouteError::internal("failed to delete mobile profile")
        })
}

pub(super) async fn list_mobile_devices_for_profile_for_route(
    state: &Arc<DaemonState>,
    profile_id: ConnectionProfileId,
) -> Result<Vec<MobileDeviceRegistration>, MobileAccessRouteError> {
    state
        .global_store()
        .list_mobile_devices(profile_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to list mobile devices: {e:?}");
            MobileAccessRouteError::internal("failed to list mobile devices")
        })
}

pub(super) async fn register_mobile_device_for_route(
    state: &Arc<DaemonState>,
    auth: MobileAuthContext,
    request: RegisterMobileDeviceForRouteRequest,
) -> Result<MobileDeviceRegistration, MobileAccessRouteError> {
    if !auth.allows(MobileScope::DeviceRegistration) {
        return Err(MobileAccessRouteError::unauthorized(
            MobileScope::DeviceRegistration.missing_error(),
        ));
    }
    let device_uuid = uuid::Uuid::parse_str(request.device_id.trim())
        .map_err(|_| MobileAccessRouteError::bad_request("device_id must be a UUID"))?;
    state
        .global_store()
        .upsert_mobile_device(
            MobileDeviceId(device_uuid),
            auth.profile_id,
            MobileDeviceRegistrationUpdate {
                device_label: sanitize_optional_mobile_field(request.device_label),
                platform: sanitize_optional_mobile_field(request.platform),
                push_token: sanitize_optional_mobile_field(request.push_token),
                push_provider: sanitize_optional_mobile_field(request.push_provider),
                public_key: sanitize_optional_mobile_field(request.public_key),
                app_version: sanitize_optional_mobile_field(request.app_version),
            }
            .into(),
        )
        .await
        .map_err(|e| {
            tracing::error!("failed to register mobile device: {e:?}");
            MobileAccessRouteError::internal("failed to register device")
        })
}

fn normalize_profile_base_url(base_url_raw: &str) -> Result<String, MobileAccessRouteError> {
    let base_url_raw = base_url_raw.trim();
    if base_url_raw.is_empty() {
        return Err(MobileAccessRouteError::bad_request("base_url is required"));
    }
    let parsed = Url::parse(base_url_raw)
        .map_err(|_| MobileAccessRouteError::bad_request("base_url must be a valid URL"))?;
    if parsed.scheme() != "https" {
        return Err(MobileAccessRouteError::bad_request(
            "base_url must use https://",
        ));
    }
    Ok(parsed.as_str().trim_end_matches('/').to_string())
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

fn sanitize_optional_mobile_field(input: Option<String>) -> Option<String> {
    input
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
