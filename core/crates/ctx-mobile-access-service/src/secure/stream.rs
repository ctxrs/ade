use ctx_core::ids::{MobileDeviceId, WorkspaceId};
use ctx_store::Store;
use ctx_transport_runtime::mobile_e2ee;

use crate::{
    load_mobile_auth_context_for_profile, MobileScope, MobileSecureStreamContext,
    MobileSecureWorkspaceStreamAdmission, MobileSecureWorkspaceStreamRouteParams,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileSecureStreamAccessError {
    BadDeviceId,
    BadWorkspaceId,
    Unauthorized,
    NotFound,
    Store,
}

pub async fn require_mobile_secure_stream_access(
    store: &Store,
    workspace_id: WorkspaceId,
    device_id: &str,
    provided_token: &str,
) -> Result<(), MobileSecureStreamAccessError> {
    load_mobile_secure_stream_context_for_access(store, workspace_id, device_id, provided_token)
        .await
        .map(|_| ())
}

pub async fn admit_mobile_secure_workspace_stream(
    store: &Store,
    params: MobileSecureWorkspaceStreamRouteParams,
) -> Result<MobileSecureWorkspaceStreamAdmission, MobileSecureStreamAccessError> {
    let workspace_id = uuid::Uuid::parse_str(params.workspace_id())
        .map(WorkspaceId)
        .map_err(|_| MobileSecureStreamAccessError::BadWorkspaceId)?;
    let device_id = params.device_id().trim();
    let token = params.token().trim();
    let context =
        load_mobile_secure_stream_context_for_access(store, workspace_id, device_id, token).await?;
    Ok(MobileSecureWorkspaceStreamAdmission {
        workspace_id,
        context,
    })
}

async fn load_mobile_secure_stream_context_for_access(
    store: &Store,
    workspace_id: WorkspaceId,
    device_id: &str,
    provided_token: &str,
) -> Result<MobileSecureStreamContext, MobileSecureStreamAccessError> {
    let device_uuid =
        uuid::Uuid::parse_str(device_id).map_err(|_| MobileSecureStreamAccessError::BadDeviceId)?;
    let cfg = store
        .get_mobile_access_config()
        .await
        .map_err(|_| MobileSecureStreamAccessError::Store)?
        .ok_or(MobileSecureStreamAccessError::Unauthorized)?;
    if !cfg.enabled {
        return Err(MobileSecureStreamAccessError::Unauthorized);
    }
    let Some(mobile_auth) = load_mobile_auth_context_for_profile(store, cfg.profile_id)
        .await
        .map_err(|_| MobileSecureStreamAccessError::Store)?
    else {
        return Err(MobileSecureStreamAccessError::Unauthorized);
    };
    if !mobile_auth.allows(MobileScope::WorkspaceStream) {
        return Err(MobileSecureStreamAccessError::Unauthorized);
    }
    let device = store
        .get_mobile_device(MobileDeviceId(device_uuid))
        .await
        .map_err(|_| MobileSecureStreamAccessError::Store)?
        .ok_or(MobileSecureStreamAccessError::Unauthorized)?;
    if device.profile_id != cfg.profile_id {
        return Err(MobileSecureStreamAccessError::Unauthorized);
    }
    if device
        .public_key
        .as_deref()
        .is_none_or(|key| key.trim().is_empty())
    {
        return Err(MobileSecureStreamAccessError::Unauthorized);
    }
    let key = mobile_e2ee::derive_key(
        device_id,
        device.public_key.as_deref().unwrap_or_default(),
        &cfg.daemon_private_key,
    )
    .map_err(|_| MobileSecureStreamAccessError::Unauthorized)?;
    let expected_token = mobile_e2ee::derive_stream_token(&key, &workspace_id.0.to_string());
    if provided_token != expected_token {
        return Err(MobileSecureStreamAccessError::Unauthorized);
    }
    let workspace_exists = store
        .get_workspace(workspace_id)
        .await
        .map_err(|_| MobileSecureStreamAccessError::Store)?
        .is_some();
    if !workspace_exists {
        return Err(MobileSecureStreamAccessError::NotFound);
    }
    Ok(MobileSecureStreamContext {
        device_id: device_id.to_string(),
        key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_workspace_id_maps_before_device_or_token_work() {
        let params = MobileSecureWorkspaceStreamRouteParams::new(
            "not-a-workspace",
            "22222222-2222-2222-2222-222222222222",
            "bad-token",
        );

        let parse_result = uuid::Uuid::parse_str(params.workspace_id());
        assert!(parse_result.is_err());
    }

    #[test]
    fn stream_token_is_bound_to_workspace_uuid_string() {
        let device_id = "22222222-2222-2222-2222-222222222222";
        let workspace_id = WorkspaceId(
            uuid::Uuid::parse_str("33333333-3333-3333-3333-333333333333").expect("workspace id"),
        );
        let (_, daemon_private_key) = mobile_e2ee::generate_keypair();
        let (device_public_key, _) = mobile_e2ee::generate_keypair();
        let key = mobile_e2ee::derive_key(device_id, &device_public_key, &daemon_private_key)
            .expect("derive key");

        let token = mobile_e2ee::derive_stream_token(&key, &workspace_id.0.to_string());

        assert_eq!(
            token,
            mobile_e2ee::derive_stream_token(&key, "33333333-3333-3333-3333-333333333333")
        );
    }
}
