use super::*;

pub(super) async fn get_mobile_access_status(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
) -> Result<Json<MobileAccessStatus>, StatusCode> {
    if mobile_auth.is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile access config: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let tunnel_status = state.transport.mobile_tunnel.status().await;
    let (enabled, tunnel_id, public_base_url, relay_base_url, daemon_public_key) = match cfg {
        Some(cfg) => (
            cfg.enabled,
            Some(cfg.tunnel_id),
            Some(cfg.public_base_url),
            Some(cfg.relay_base_url),
            Some(cfg.daemon_public_key),
        ),
        None => (false, None, None, None, None),
    };
    Ok(Json(MobileAccessStatus {
        enabled,
        tunnel_id,
        public_base_url,
        relay_base_url,
        daemon_public_key,
        tunnel_state: tunnel_status.state,
        last_error: tunnel_status.last_error,
    }))
}

pub(super) async fn enable_mobile_access(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<EnableMobileAccessReq>,
) -> Result<Json<EnableMobileAccessResp>, (StatusCode, Json<ApiErrorResp>)> {
    if mobile_auth.is_some() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "desktop auth required".into(),
            }),
        ));
    }
    if state.core.auth_token.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "daemon auth token is not configured; refusing to expose daemon publicly"
                    .into(),
            }),
        ));
    }

    let control_plane_url = resolve_control_plane_url();
    if control_plane_url.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "CTX_TUNNEL_CONTROL_PLANE_URL is not set".into(),
            }),
        ));
    }

    let enable_resp = reqwest::Client::new()
        .post(format!(
            "{}/v1/mobile/enable",
            control_plane_url.trim_end_matches('/')
        ))
        .bearer_auth(req.supabase_token.trim())
        .send()
        .await
        .map_err(|e| {
            tracing::error!("failed to call control plane: {e:?}");
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiErrorResp {
                    error: "failed to reach control plane".into(),
                }),
            )
        })?;

    if !enable_resp.status().is_success() {
        let status = enable_resp.status();
        let body = enable_resp.text().await.unwrap_or_default();
        tracing::warn!("control plane denied enable: {status} {body}");
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiErrorResp {
                error: "mobile access not entitled".into(),
            }),
        ));
    }

    let payload = enable_resp
        .json::<ControlPlaneEnableResp>()
        .await
        .map_err(|e| {
            tracing::error!("invalid control plane response: {e:?}");
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiErrorResp {
                    error: "invalid control plane response".into(),
                }),
            )
        })?;

    let public_url = Url::parse(&payload.public_base_url).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "public_base_url must be a valid URL".into(),
            }),
        )
    })?;
    if public_url.scheme() != "https" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "public_base_url must use https://".into(),
            }),
        ));
    }

    let now = chrono::Utc::now();
    let (daemon_public_key, daemon_private_key, profile_id, created_at) = match state
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
        Some(cfg) => (
            cfg.daemon_public_key,
            cfg.daemon_private_key,
            cfg.profile_id,
            cfg.created_at,
        ),
        None => {
            let (public_key, private_key) = crate::mobile_e2ee::generate_keypair();
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
                    Vec::new(),
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
            (public_key, private_key, profile.id, now)
        }
    };

    let config = ctx_store::store::MobileAccessConfig {
        id: "default".to_string(),
        profile_id,
        tunnel_id: payload.tunnel_id.clone(),
        public_base_url: public_url.as_str().trim_end_matches('/').to_string(),
        relay_base_url: payload.relay_base_url.clone(),
        tunnel_secret: payload.tunnel_secret.clone(),
        daemon_public_key: daemon_public_key.clone(),
        daemon_private_key: daemon_private_key.clone(),
        enabled: true,
        created_at,
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

    let pairing_token = generate_pairing_token();
    let pairing_hash = hash_pairing_token(&pairing_token);
    let expires_at = now + chrono::Duration::seconds(PAIRING_TOKEN_TTL_SECS);
    state
        .global_store()
        .insert_mobile_pairing_token(&uuid::Uuid::new_v4().to_string(), &pairing_hash, expires_at)
        .await
        .map_err(|e| {
            tracing::error!("failed to persist pairing token: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to persist pairing token".into(),
                }),
            )
        })?;

    let tunnel_cfg = crate::mobile_tunnel::StartMobileTunnelConfig {
        relay_base_url: payload.relay_base_url.clone(),
        tunnel_id: payload.tunnel_id.clone(),
        tunnel_secret: payload.tunnel_secret.clone(),
        public_base_url: public_url.as_str().trim_end_matches('/').to_string(),
        local_daemon_url: state.core.daemon_url.trim_end_matches('/').to_string(),
    };
    if let Err(e) = state.transport.mobile_tunnel.start(tunnel_cfg).await {
        tracing::warn!("failed to start mobile tunnel: {e:#}");
    }

    let status = MobileAccessStatus {
        enabled: true,
        tunnel_id: Some(payload.tunnel_id.clone()),
        public_base_url: Some(public_url.as_str().trim_end_matches('/').to_string()),
        relay_base_url: Some(payload.relay_base_url.clone()),
        daemon_public_key: Some(daemon_public_key.clone()),
        tunnel_state: crate::mobile_tunnel::MobileTunnelState::Running,
        last_error: None,
    };

    let qr_payload = serde_json::json!({
        "type": "context_mobile_e2ee",
        "version": 1,
        "tunnel_id": payload.tunnel_id,
        "base_url": public_url.as_str().trim_end_matches('/'),
        "pairing_token": pairing_token,
        "daemon_public_key": daemon_public_key,
    });

    Ok(Json(EnableMobileAccessResp {
        status,
        qr_payload,
        pairing_expires_at: expires_at.to_rfc3339(),
    }))
}

pub(super) async fn disable_mobile_access(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<EnableMobileAccessReq>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    if mobile_auth.is_some() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "desktop auth required".into(),
            }),
        ));
    }

    let control_plane_url = resolve_control_plane_url();
    if !control_plane_url.trim().is_empty() {
        let _ = reqwest::Client::new()
            .post(format!(
                "{}/v1/mobile/revoke",
                control_plane_url.trim_end_matches('/')
            ))
            .bearer_auth(req.supabase_token.trim())
            .send()
            .await;
    }

    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|e| {
            tracing::error!(
                "failed to read mobile access config while disabling mobile access: {e:?}"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to read mobile access config".into(),
                }),
            )
        })?;

    state.transport.mobile_tunnel.stop().await;
    state
        .global_store()
        .clear_mobile_pairing_tokens()
        .await
        .map_err(|e| {
            tracing::error!("failed to clear pairing tokens while disabling mobile access: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to clear pairing tokens".into(),
                }),
            )
        })?;
    if let Some(cfg) = cfg {
        state
            .global_store()
            .delete_mobile_access_config()
            .await
            .map_err(|e| {
                tracing::error!(
                    "failed to delete mobile access config while disabling mobile access: {e:?}"
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: "failed to delete mobile access config".into(),
                    }),
                )
            })?;
        state
            .global_store()
            .delete_mobile_connection_profile(cfg.profile_id)
            .await
            .map_err(|e| {
                tracing::error!(
                    "failed to delete mobile connection profile while disabling mobile access: {e:?}"
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: "failed to delete mobile connection profile".into(),
                    }),
                )
            })?;
    }
    Ok(StatusCode::NO_CONTENT)
}
