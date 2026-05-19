use std::sync::Arc;

use ctx_core::ids::{MobileDeviceId, WorkspaceId};
use ctx_transport_runtime::mobile_e2ee::{self, E2eeKey};

use super::{
    load_mobile_auth_context_for_profile, MobileAccessRouteError, MobileAccessRouteErrorKind,
    MobileScope,
};
use crate::daemon::DaemonState;

#[derive(Clone)]
pub struct MobileSecureStreamContext {
    pub device_id: String,
    pub key: E2eeKey,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MobileSecureWorkspaceStreamRouteParams {
    workspace_id: String,
    device_id: String,
    token: String,
}

impl MobileSecureWorkspaceStreamRouteParams {
    pub fn new(
        workspace_id: impl Into<String>,
        device_id: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            device_id: device_id.into(),
            token: token.into(),
        }
    }
}

#[derive(Clone)]
pub struct MobileSecureWorkspaceStreamAdmission {
    pub workspace_id: WorkspaceId,
    pub context: MobileSecureStreamContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileSecureStreamAccessError {
    BadDeviceId,
    Unauthorized,
    NotFound,
    Store,
}

pub async fn require_mobile_secure_stream_access(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    device_id: &str,
    provided_token: &str,
) -> Result<(), MobileSecureStreamAccessError> {
    load_mobile_secure_stream_context_for_access(state, workspace_id, device_id, provided_token)
        .await
        .map(|_| ())
}

pub(super) async fn admit_mobile_secure_workspace_stream_for_route(
    state: &Arc<DaemonState>,
    params: MobileSecureWorkspaceStreamRouteParams,
) -> Result<MobileSecureWorkspaceStreamAdmission, MobileAccessRouteError> {
    let workspace_id = uuid::Uuid::parse_str(&params.workspace_id)
        .map(WorkspaceId)
        .map_err(|_| MobileAccessRouteError::bad_request("invalid workspace id"))?;
    let device_id = params.device_id.trim();
    let token = params.token.trim();
    let context =
        load_mobile_secure_stream_context_for_access(state, workspace_id, device_id, token)
            .await
            .map_err(mobile_secure_stream_access_route_error)?;
    Ok(MobileSecureWorkspaceStreamAdmission {
        workspace_id,
        context,
    })
}

async fn load_mobile_secure_stream_context_for_access(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    device_id: &str,
    provided_token: &str,
) -> Result<MobileSecureStreamContext, MobileSecureStreamAccessError> {
    let device_uuid =
        uuid::Uuid::parse_str(device_id).map_err(|_| MobileSecureStreamAccessError::BadDeviceId)?;
    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|_| MobileSecureStreamAccessError::Store)?
        .ok_or(MobileSecureStreamAccessError::Unauthorized)?;
    if !cfg.enabled {
        return Err(MobileSecureStreamAccessError::Unauthorized);
    }
    let Some(mobile_auth) = load_mobile_auth_context_for_profile(state, cfg.profile_id)
        .await
        .map_err(|_| MobileSecureStreamAccessError::Store)?
    else {
        return Err(MobileSecureStreamAccessError::Unauthorized);
    };
    if !mobile_auth.allows(MobileScope::WorkspaceStream) {
        return Err(MobileSecureStreamAccessError::Unauthorized);
    }
    let device = state
        .global_store()
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
    let workspace_exists = state
        .global_store()
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

fn mobile_secure_stream_access_route_error(
    error: MobileSecureStreamAccessError,
) -> MobileAccessRouteError {
    match error {
        MobileSecureStreamAccessError::BadDeviceId => {
            MobileAccessRouteError::bad_request("device_id must be a UUID")
        }
        MobileSecureStreamAccessError::Unauthorized => {
            MobileAccessRouteError::unauthorized(MobileScope::WorkspaceStream.missing_error())
        }
        MobileSecureStreamAccessError::NotFound => {
            MobileAccessRouteError::new(MobileAccessRouteErrorKind::NotFound, "workspace not found")
        }
        MobileSecureStreamAccessError::Store => {
            MobileAccessRouteError::internal("failed to authorize mobile stream")
        }
    }
}

pub async fn load_mobile_secure_stream_context(
    state: &Arc<DaemonState>,
    device_id: String,
) -> Result<MobileSecureStreamContext, anyhow::Error> {
    let device_uuid = uuid::Uuid::parse_str(&device_id)?;
    let config = state.global_store().get_mobile_access_config().await?;
    let config = config.ok_or_else(|| anyhow::anyhow!("mobile access not configured"))?;
    if !config.enabled {
        return Err(anyhow::anyhow!("mobile access not enabled"));
    }

    let Some(mobile_auth) = load_mobile_auth_context_for_profile(state, config.profile_id)
        .await
        .map_err(|_| anyhow::anyhow!("failed to load mobile access profile"))?
    else {
        return Err(anyhow::anyhow!(
            "{}",
            MobileScope::WorkspaceStream.missing_error()
        ));
    };
    if !mobile_auth.allows(MobileScope::WorkspaceStream) {
        return Err(anyhow::anyhow!(
            "{}",
            MobileScope::WorkspaceStream.missing_error()
        ));
    }

    let device = state
        .global_store()
        .get_mobile_device(MobileDeviceId(device_uuid))
        .await?
        .ok_or_else(|| anyhow::anyhow!("device not registered"))?;
    if device.profile_id != config.profile_id {
        return Err(anyhow::anyhow!("device not authorized for tunnel"));
    }
    let device_public_key = device
        .public_key
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("device missing public key"))?;
    let key = mobile_e2ee::derive_key(&device_id, device_public_key, &config.daemon_private_key)?;

    Ok(MobileSecureStreamContext { device_id, key })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::mobile_access::MobileAccessRouteErrorKind;
    use crate::test_support::TestDaemon;
    use tempfile::tempdir;

    #[tokio::test]
    async fn mobile_secure_workspace_stream_route_rejects_invalid_workspace_id() {
        let temp = tempdir().expect("tempdir");
        let daemon = TestDaemon::new_for_test(
            temp.path().to_path_buf(),
            "http://127.0.0.1:4567".to_string(),
        )
        .await
        .expect("test daemon");

        let result = daemon
            .handle()
            .core()
            .admit_mobile_secure_workspace_stream_for_route(
                MobileSecureWorkspaceStreamRouteParams::new(
                    "not-a-workspace",
                    "22222222-2222-2222-2222-222222222222",
                    "bad-token",
                ),
            )
            .await;
        let error = match result {
            Ok(_) => panic!("invalid workspace id should reject route admission"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), MobileAccessRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid workspace id");
    }

    #[test]
    fn mobile_secure_stream_access_errors_map_to_route_errors() {
        let bad_device =
            mobile_secure_stream_access_route_error(MobileSecureStreamAccessError::BadDeviceId);
        assert_eq!(bad_device.kind(), MobileAccessRouteErrorKind::BadRequest);
        assert_eq!(bad_device.message(), "device_id must be a UUID");

        let unauthorized =
            mobile_secure_stream_access_route_error(MobileSecureStreamAccessError::Unauthorized);
        assert_eq!(
            unauthorized.kind(),
            MobileAccessRouteErrorKind::Unauthorized
        );

        let not_found =
            mobile_secure_stream_access_route_error(MobileSecureStreamAccessError::NotFound);
        assert_eq!(not_found.kind(), MobileAccessRouteErrorKind::NotFound);
        assert_eq!(not_found.message(), "workspace not found");

        let store = mobile_secure_stream_access_route_error(MobileSecureStreamAccessError::Store);
        assert_eq!(store.kind(), MobileAccessRouteErrorKind::Internal);
        assert_eq!(store.message(), "failed to authorize mobile stream");
    }
}
