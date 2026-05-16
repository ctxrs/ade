use super::*;
use managed_profile::{create_managed_mobile_access_keys, ensure_managed_profile_scopes};

#[path = "profile_config/managed_profile.rs"]
mod managed_profile;

pub(super) struct ManagedMobileAccessKeys {
    pub(super) daemon_public_key: String,
    pub(super) daemon_private_key: String,
    pub(super) profile_id: ConnectionProfileId,
    pub(super) created_at: chrono::DateTime<chrono::Utc>,
}

pub(super) async fn load_or_create_managed_mobile_access_keys(
    state: &CoreHandle,
    public_url: &Url,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<ManagedMobileAccessKeys, (StatusCode, Json<ApiErrorResp>)> {
    match state.get_mobile_access_config().await.map_err(|e| {
        tracing::error!("failed to read mobile access config: {e:?}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: "failed to read mobile access config".into(),
            }),
        )
    })? {
        Some(cfg) => {
            ensure_managed_profile_scopes(state, cfg.profile_id).await?;
            Ok(ManagedMobileAccessKeys {
                daemon_public_key: cfg.daemon_public_key,
                daemon_private_key: cfg.daemon_private_key,
                profile_id: cfg.profile_id,
                created_at: cfg.created_at,
            })
        }
        None => create_managed_mobile_access_keys(state, public_url, now).await,
    }
}

pub(super) async fn persist_mobile_access_config(
    state: &CoreHandle,
    payload: &ControlPlaneEnableResp,
    public_url: &Url,
    keys: &ManagedMobileAccessKeys,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), (StatusCode, Json<ApiErrorResp>)> {
    let config = MobileAccessConfigUpsert {
        profile_id: keys.profile_id,
        tunnel_id: payload.tunnel_id.clone(),
        public_base_url: public_url.as_str().trim_end_matches('/').to_string(),
        relay_base_url: payload.relay_base_url.clone(),
        tunnel_secret: payload.tunnel_secret.clone(),
        daemon_public_key: keys.daemon_public_key.clone(),
        daemon_private_key: keys.daemon_private_key.clone(),
        enabled: true,
        created_at: keys.created_at,
        updated_at: now,
    };

    state
        .upsert_mobile_access_config(config)
        .await
        .map_err(|e| {
            tracing::error!("failed to persist mobile access config: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to persist mobile access config".into(),
                }),
            )
        })?;

    Ok(())
}
