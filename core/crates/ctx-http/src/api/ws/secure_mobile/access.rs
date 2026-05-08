use super::*;

pub(super) async fn require_mobile_secure_stream_access(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    device_id: &str,
    provided_token: &str,
) -> Result<(), StatusCode> {
    let device_uuid = uuid::Uuid::parse_str(device_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if !cfg.enabled {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let Some(mobile_auth) = load_mobile_auth_context_for_profile(state, cfg.profile_id).await?
    else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    if !mobile_auth.allows(MobileScope::WorkspaceStream) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let device = state
        .global_store()
        .get_mobile_device(MobileDeviceId(device_uuid))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if device.profile_id != cfg.profile_id {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if device
        .public_key
        .as_deref()
        .is_none_or(|key| key.trim().is_empty())
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let key = mobile_e2ee::derive_key(
        device_id,
        device.public_key.as_deref().unwrap_or_default(),
        &cfg.daemon_private_key,
    )
    .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let expected_token = mobile_e2ee::derive_stream_token(&key, &workspace_id.0.to_string());
    if provided_token != expected_token {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let workspace_exists = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some();
    if !workspace_exists {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(())
}
