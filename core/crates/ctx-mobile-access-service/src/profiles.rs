use url::Url;

use ctx_core::ids::{ConnectionProfileId, MobileDeviceId};
use ctx_core::models::{MobileConnectionProfile, MobileDeviceRegistration};
use ctx_store::Store;

use crate::tokens::{generate_mobile_api_token, hash_api_token};
use crate::{
    mobile_scope_set_from_strings, MobileAccessServiceError, MobileAuthContext,
    MobileDeviceRegistrationUpdate, MobileScope,
};

#[derive(Debug, Clone)]
pub struct CreateMobileConnectionProfileRequest {
    pub label: String,
    pub base_url: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CreateMobileConnectionProfileResult {
    pub profile: MobileConnectionProfile,
    pub token: String,
}

#[derive(Debug, Clone, Default)]
pub struct RegisterMobileDeviceRequest {
    pub device_id: String,
    pub device_label: Option<String>,
    pub platform: Option<String>,
    pub push_token: Option<String>,
    pub push_provider: Option<String>,
    pub public_key: Option<String>,
    pub app_version: Option<String>,
}

pub async fn create_mobile_connection_profile(
    store: &Store,
    request: CreateMobileConnectionProfileRequest,
) -> Result<CreateMobileConnectionProfileResult, MobileAccessServiceError> {
    let label = request.label.trim();
    if label.is_empty() {
        return Err(MobileAccessServiceError::bad_request("label is required"));
    }
    let normalized_base = normalize_profile_base_url(&request.base_url)?;
    let scopes = mobile_scope_set_from_strings(&request.scopes)
        .map(|scope_set| scope_set.to_strings())
        .map_err(MobileAccessServiceError::bad_request)?;
    let token = generate_mobile_api_token();
    let token_hash = hash_api_token(&token);
    let token_prefix: String = token.chars().take(8).collect();
    let profile = store
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
            MobileAccessServiceError::internal("failed to create profile")
        })?;
    Ok(CreateMobileConnectionProfileResult { profile, token })
}

pub async fn list_mobile_connection_profiles(
    store: &Store,
) -> Result<Vec<MobileConnectionProfile>, MobileAccessServiceError> {
    store.list_mobile_connection_profiles().await.map_err(|e| {
        tracing::error!("failed to list mobile profiles: {e:?}");
        MobileAccessServiceError::internal("failed to list mobile profiles")
    })
}

pub async fn delete_mobile_connection_profile(
    store: &Store,
    profile_id: ConnectionProfileId,
) -> Result<(), MobileAccessServiceError> {
    if store
        .get_mobile_connection_profile(profile_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to load mobile profile before delete: {e:?}");
            MobileAccessServiceError::internal("failed to load mobile profile")
        })?
        .is_none()
    {
        return Err(MobileAccessServiceError::not_found(
            "mobile profile not found",
        ));
    }
    store
        .delete_mobile_connection_profile(profile_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to delete mobile profile: {e:?}");
            MobileAccessServiceError::internal("failed to delete mobile profile")
        })
}

pub async fn list_mobile_devices_for_profile(
    store: &Store,
    profile_id: ConnectionProfileId,
) -> Result<Vec<MobileDeviceRegistration>, MobileAccessServiceError> {
    store.list_mobile_devices(profile_id).await.map_err(|e| {
        tracing::error!("failed to list mobile devices: {e:?}");
        MobileAccessServiceError::internal("failed to list mobile devices")
    })
}

pub async fn register_mobile_device(
    store: &Store,
    auth: MobileAuthContext,
    request: RegisterMobileDeviceRequest,
) -> Result<MobileDeviceRegistration, MobileAccessServiceError> {
    if !auth.allows(MobileScope::DeviceRegistration) {
        return Err(MobileAccessServiceError::unauthorized(
            MobileScope::DeviceRegistration.missing_error(),
        ));
    }
    let device_uuid = uuid::Uuid::parse_str(request.device_id.trim())
        .map_err(|_| MobileAccessServiceError::bad_request("device_id must be a UUID"))?;
    store
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
            MobileAccessServiceError::internal("failed to register device")
        })
}

fn normalize_profile_base_url(base_url_raw: &str) -> Result<String, MobileAccessServiceError> {
    let base_url_raw = base_url_raw.trim();
    if base_url_raw.is_empty() {
        return Err(MobileAccessServiceError::bad_request(
            "base_url is required",
        ));
    }
    let parsed = Url::parse(base_url_raw)
        .map_err(|_| MobileAccessServiceError::bad_request("base_url must be a valid URL"))?;
    if parsed.scheme() != "https" {
        return Err(MobileAccessServiceError::bad_request(
            "base_url must use https://",
        ));
    }
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

fn sanitize_optional_mobile_field(input: Option<String>) -> Option<String> {
    input
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{default_mobile_profile_scopes, hash_api_token, resolve_mobile_auth_context};
    use ctx_store::Store;

    async fn test_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("mobile-access-service.sqlite");
        let store = Store::open(&db_path).await.expect("open store");
        (dir, store)
    }

    #[tokio::test]
    async fn create_mobile_connection_profile_trims_and_persists_profile() {
        let (_dir, store) = test_store().await;
        let result = create_mobile_connection_profile(
            &store,
            CreateMobileConnectionProfileRequest {
                label: "  Field phone  ".to_string(),
                base_url: "https://daemon.example.test/".to_string(),
                scopes: vec!["workspace_stream".to_string()],
            },
        )
        .await
        .expect("create profile");

        assert_eq!(result.profile.label, "Field phone");
        assert_eq!(result.profile.base_url, "https://daemon.example.test");
        assert_eq!(result.profile.scopes, vec!["workspace_stream"]);
        assert_eq!(
            store
                .get_mobile_connection_profile_by_token_hash(&hash_api_token(&result.token))
                .await
                .expect("load by token")
                .expect("stored profile")
                .id,
            result.profile.id
        );
    }

    #[tokio::test]
    async fn create_mobile_connection_profile_rejects_non_https_base_url() {
        let (_dir, store) = test_store().await;
        let error = create_mobile_connection_profile(
            &store,
            CreateMobileConnectionProfileRequest {
                label: "Field phone".to_string(),
                base_url: "http://daemon.example.test".to_string(),
                scopes: vec![],
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.kind(),
            crate::MobileAccessServiceErrorKind::BadRequest
        );
        assert_eq!(error.message(), "base_url must use https://");
    }

    #[tokio::test]
    async fn register_mobile_device_sanitizes_optional_fields() {
        let (_dir, store) = test_store().await;
        let profile = store
            .create_mobile_connection_profile(
                "Field phone".to_string(),
                "https://daemon.example.test".to_string(),
                "token-hash".to_string(),
                "token-pr".to_string(),
                default_mobile_profile_scopes(),
            )
            .await
            .expect("create profile");
        let auth = resolve_mobile_auth_context(&store, profile)
            .await
            .expect("resolve auth")
            .expect("auth context");
        let device_id = uuid::Uuid::new_v4().to_string();

        let device = register_mobile_device(
            &store,
            auth,
            RegisterMobileDeviceRequest {
                device_id: device_id.clone(),
                device_label: Some("  iPhone  ".to_string()),
                platform: Some(" ios ".to_string()),
                push_token: Some("   ".to_string()),
                push_provider: None,
                public_key: Some(" pubkey ".to_string()),
                app_version: Some(" 1.2.3 ".to_string()),
            },
        )
        .await
        .expect("register device");

        assert_eq!(device.id.0.to_string(), device_id);
        assert_eq!(device.profile_id, auth.profile_id);
        assert_eq!(device.device_label.as_deref(), Some("iPhone"));
        assert_eq!(device.platform.as_deref(), Some("ios"));
        assert_eq!(device.push_token, None);
        assert_eq!(device.public_key.as_deref(), Some("pubkey"));
        assert_eq!(device.app_version.as_deref(), Some("1.2.3"));
    }
}
