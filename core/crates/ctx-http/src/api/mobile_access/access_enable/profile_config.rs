use super::*;

pub(super) struct ManagedMobileAccessKeys {
    pub(super) daemon_public_key: String,
    pub(super) daemon_private_key: String,
    pub(super) profile_id: ConnectionProfileId,
    pub(super) created_at: chrono::DateTime<chrono::Utc>,
}

pub(super) async fn load_or_create_managed_mobile_access_keys(
    state: &Arc<AppState>,
    public_url: &Url,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<ManagedMobileAccessKeys, (StatusCode, Json<ApiErrorResp>)> {
    match state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|e| {
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
    state: &Arc<AppState>,
    payload: &ControlPlaneEnableResp,
    public_url: &Url,
    keys: &ManagedMobileAccessKeys,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), (StatusCode, Json<ApiErrorResp>)> {
    let config = ctx_store::store::MobileAccessConfig {
        id: "default".to_string(),
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
        .global_store()
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

async fn ensure_managed_profile_scopes(
    state: &Arc<AppState>,
    profile_id: ConnectionProfileId,
) -> Result<(), (StatusCode, Json<ApiErrorResp>)> {
    let profile = state
        .global_store()
        .get_mobile_connection_profile(profile_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to read managed mobile profile: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to read managed profile".into(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "managed mobile profile is missing".into(),
                }),
            )
        })?;
    if profile.scopes.is_empty() {
        state
            .global_store()
            .update_mobile_connection_profile_scopes(profile.id, default_mobile_profile_scopes())
            .await
            .map_err(|e| {
                tracing::error!("failed to backfill managed mobile profile scopes: {e:?}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: "failed to update managed profile".into(),
                    }),
                )
            })?;
    }
    Ok(())
}

async fn create_managed_mobile_access_keys(
    state: &Arc<AppState>,
    public_url: &Url,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<ManagedMobileAccessKeys, (StatusCode, Json<ApiErrorResp>)> {
    let (public_key, private_key) = mobile_e2ee::generate_keypair();
    let token = generate_mobile_api_token();
    let token_hash = hash_api_token(&token);
    let token_prefix: String = token.chars().take(8).collect();
    let profile = state
        .global_store()
        .create_mobile_connection_profile(
            "Managed Mobile Access".to_string(),
            public_url.as_str().trim_end_matches('/').to_string(),
            token_hash,
            token_prefix,
            default_mobile_profile_scopes(),
        )
        .await
        .map_err(|e| {
            tracing::error!("failed to create managed mobile profile: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to create managed profile".into(),
                }),
            )
        })?;
    Ok(ManagedMobileAccessKeys {
        daemon_public_key: public_key,
        daemon_private_key: private_key,
        profile_id: profile.id,
        created_at: now,
    })
}
