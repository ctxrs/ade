use super::*;
mod body;
mod control_plane;
mod payloads;
mod profiles;
use body::{decode_body_b64, parse_json_body};
use control_plane::{resolve_control_plane_url, PAIRING_TOKEN_TTL_SECS};
pub(in crate::api) use payloads::*;
pub(in crate::api) use profiles::{
    create_mobile_connection_profile, delete_mobile_connection_profile,
    list_mobile_connection_profiles, list_mobile_devices_for_profile, register_mobile_device,
};

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

    let cfg = state.global_store().get_mobile_access_config().await.map_err(|e| {
        tracing::error!("failed to read mobile access config while disabling mobile access: {e:?}");
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
        state.global_store().delete_mobile_access_config().await.map_err(|e| {
            tracing::error!("failed to delete mobile access config while disabling mobile access: {e:?}");
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

pub(super) async fn pair_mobile_device(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Result<Json<SecureEnvelope>, (StatusCode, Json<ApiErrorResp>)> {
    let req: PairMobileDeviceReq = parse_json_body(body)?;
    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile access config: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "mobile access not configured".into(),
                }),
            )
        })?;
    let Some(cfg) = cfg else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "mobile access not enabled".into(),
            }),
        ));
    };
    if !cfg.enabled {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "mobile access not enabled".into(),
            }),
        ));
    }

    let device_uuid = uuid::Uuid::parse_str(req.device_id.trim()).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "device_id must be a UUID".into(),
            }),
        )
    })?;

    let token_hash = hash_pairing_token(req.pairing_token.trim());
    let key = crate::mobile_e2ee::derive_key(
        &req.device_id,
        req.public_key.trim(),
        &cfg.daemon_private_key,
    )
    .map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "failed to derive pairing key".into(),
            }),
        )
    })?;
    let allowed = state
        .global_store()
        .consume_mobile_pairing_token(&token_hash)
        .await
        .map_err(|e| {
            tracing::error!("failed to check pairing token: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to validate pairing token".into(),
                }),
            )
        })?;
    if !allowed {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "pairing token invalid or expired".into(),
            }),
        ));
    }
    let _device = state
        .global_store()
        .upsert_mobile_device(
            MobileDeviceId(device_uuid),
            cfg.profile_id,
            MobileDeviceUpsert {
                device_label: req
                    .device_label
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                platform: req
                    .platform
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                push_token: None,
                push_provider: None,
                public_key: Some(req.public_key.trim().to_string()),
                app_version: req
                    .app_version
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
            },
        )
        .await
        .map_err(|e| {
            tracing::error!("failed to register device: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to register device".into(),
                }),
            )
        })?;

    let payload = serde_json::json!({
        "paired": true,
        "device_id": req.device_id,
        "daemon_public_key": cfg.daemon_public_key,
        "paired_at": chrono::Utc::now().to_rfc3339(),
    });
    let plaintext = serde_json::to_vec(&payload).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: "failed to encode pairing response".into(),
            }),
        )
    })?;
    let envelope =
        crate::mobile_e2ee::encrypt(&key, &req.device_id, 0, &plaintext).map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to encrypt pairing response".into(),
                }),
            )
        })?;

    Ok(Json(SecureEnvelope {
        device_id: envelope.device_id,
        seq: envelope.seq,
        nonce: envelope.nonce_b64,
        ciphertext: envelope.ciphertext_b64,
    }))
}

pub(super) async fn handle_mobile_secure(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Result<Json<SecureEnvelope>, (StatusCode, Json<ApiErrorResp>)> {
    let req: MobileSecureEnvelope = parse_json_body(body)?;
    let device_uuid = uuid::Uuid::parse_str(req.device_id.trim()).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "device_id must be a UUID".into(),
            }),
        )
    })?;
    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile access config: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "mobile access not configured".into(),
                }),
            )
        })?;
    let Some(cfg) = cfg else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "mobile access not enabled".into(),
            }),
        ));
    };
    if !cfg.enabled {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "mobile access not enabled".into(),
            }),
        ));
    }
    let device = state
        .global_store()
        .get_mobile_device(MobileDeviceId(device_uuid))
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile device: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to read device".into(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResp {
                    error: "unknown device".into(),
                }),
            )
        })?;
    if device.profile_id != cfg.profile_id {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "device not authorized for this tunnel".into(),
            }),
        ));
    }

    let Some(device_public_key) = device.public_key.as_ref() else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "device missing public key".into(),
            }),
        ));
    };
    let key =
        crate::mobile_e2ee::derive_key(&req.device_id, device_public_key, &cfg.daemon_private_key)
            .map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: "failed to derive device key".into(),
                    }),
                )
            })?;

    let plaintext =
        crate::mobile_e2ee::decrypt(&key, &req.device_id, req.seq, &req.nonce, &req.ciphertext)
            .map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: "failed to decrypt request".into(),
                    }),
                )
            })?;

    let payload: SecureRequestPayload = serde_json::from_slice(&plaintext).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid secure request payload".into(),
            }),
        )
    })?;

    let normalized_path = payload.path.trim();
    if normalized_path.starts_with("/api/mobile/secure")
        || normalized_path.starts_with("/api/mobile/pair")
        || normalized_path.starts_with("/api/mobile/")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "secure proxy cannot target mobile management endpoints".into(),
            }),
        ));
    }

    match state
        .global_store()
        .advance_mobile_device_seq(MobileDeviceId(device_uuid), req.seq)
        .await
        .map_err(|e| {
            tracing::error!("failed to update device seq: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to update device".into(),
                }),
            )
        })? {
        ctx_store::store::MobileDeviceSeqAdvance::Advanced => {}
        ctx_store::store::MobileDeviceSeqAdvance::Stale { current } => {
            tracing::warn!(device_id = %device_uuid, seq = req.seq, current, "rejected stale mobile secure request");
            return Err((
                StatusCode::CONFLICT,
                Json(ApiErrorResp {
                    error: "stale request sequence".into(),
                }),
            ));
        }
        ctx_store::store::MobileDeviceSeqAdvance::Missing => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "device not registered".into(),
                }),
            ));
        }
    }

    let response_payload = proxy_secure_request(&state, device.profile_id, payload)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(ApiErrorResp { error: e })))?;

    let response_bytes = serde_json::to_vec(&response_payload).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: "failed to encode secure response".into(),
            }),
        )
    })?;
    let envelope = crate::mobile_e2ee::encrypt(&key, &req.device_id, req.seq, &response_bytes)
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to encrypt response".into(),
                }),
            )
        })?;

    Ok(Json(SecureEnvelope {
        device_id: envelope.device_id,
        seq: envelope.seq,
        nonce: envelope.nonce_b64,
        ciphertext: envelope.ciphertext_b64,
    }))
}

pub(super) async fn proxy_secure_request(
    state: &Arc<AppState>,
    profile_id: ConnectionProfileId,
    mut payload: SecureRequestPayload,
) -> Result<SecureResponsePayload, String> {
    if let Some((path, query)) = payload.path.split_once('?') {
        let path = path.to_string();
        let query = query.to_string();
        payload.path = path;
        if payload.query.is_none() {
            payload.query = Some(query);
        }
    }
    let path = payload.path.trim().to_string();
    if !path.starts_with("/api/") {
        return Err("secure proxy only supports /api/* paths".to_string());
    }

    let method = axum::http::Method::from_bytes(payload.method.as_bytes())
        .map_err(|_| "invalid http method".to_string())?;
    let mut uri = path;
    if let Some(query) = payload
        .query
        .as_ref()
        .map(|q| q.trim())
        .filter(|q| !q.is_empty())
    {
        uri.push('?');
        uri.push_str(query.trim_start_matches('?'));
    }

    let body = decode_body_b64(&payload.body_b64)?;
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in payload.headers {
        if name.eq_ignore_ascii_case("host") || name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        let Ok(header_name) = header::HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        let Ok(header_value) = header::HeaderValue::from_str(&value) else {
            continue;
        };
        builder = builder.header(header_name, header_value);
    }

    let mut req = builder
        .body(Body::from(body))
        .map_err(|_| "failed to build proxied request".to_string())?;
    req.extensions_mut()
        .insert(MobileAuthContext { profile_id });

    let app = router(state.clone());
    let resp = app
        .oneshot(req)
        .await
        .map_err(|_| "failed to proxy request".to_string())?;

    let status = resp.status().as_u16();
    let headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .filter_map(|(k, v)| Some((k.to_string(), v.to_str().ok()?.to_string())))
        .collect();
    let body_bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024 * 1024)
        .await
        .map_err(|_| "failed to read proxied response".to_string())?;
    let body_b64 = base64::engine::general_purpose::STANDARD.encode(body_bytes);
    Ok(SecureResponsePayload {
        status,
        headers,
        body_b64,
    })
}
