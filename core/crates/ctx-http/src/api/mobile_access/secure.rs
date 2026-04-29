use super::*;

pub(in crate::api) async fn pair_mobile_device(
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
    let Some(mobile_auth) = load_mobile_auth_context_for_profile(&state, cfg.profile_id)
        .await
        .map_err(|status| {
            (
                status,
                Json(ApiErrorResp {
                    error: "failed to read mobile access profile".into(),
                }),
            )
        })?
    else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: MobileScope::DeviceRegistration.missing_error().into(),
            }),
        ));
    };
    if !mobile_auth.allows(MobileScope::DeviceRegistration) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: MobileScope::DeviceRegistration.missing_error().into(),
            }),
        ));
    }

    let device_id = req.device_id.trim().to_string();
    let device_public_key = req.public_key.trim().to_string();
    let device_uuid = uuid::Uuid::parse_str(&device_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "device_id must be a UUID".into(),
            }),
        )
    })?;

    let key =
        crate::mobile_e2ee::derive_key(&device_id, &device_public_key, &cfg.daemon_private_key)
            .map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: "failed to derive pairing key".into(),
                    }),
                )
            })?;
    if req.seq != 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "pairing seq must be 0".into(),
            }),
        ));
    }
    let decrypted = crate::mobile_e2ee::decrypt_pairing_request(
        &key,
        &device_id,
        &device_public_key,
        &req.nonce,
        &req.ciphertext,
    )
    .map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "failed to decrypt pairing request".into(),
            }),
        )
    })?;
    let payload: PairMobileDevicePayload = serde_json::from_slice(&decrypted).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid pairing request payload".into(),
            }),
        )
    })?;
    let token_hash = hash_pairing_token(payload.pairing_token.trim());
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
                device_label: payload
                    .device_label
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                platform: payload
                    .platform
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                push_token: None,
                push_provider: None,
                public_key: Some(device_public_key),
                app_version: payload
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
        "device_id": device_id.clone(),
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
    let envelope = crate::mobile_e2ee::encrypt(&key, &device_id, 0, &plaintext).map_err(|_| {
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

pub(in crate::api) async fn handle_mobile_secure(
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

    let response_payload = match load_mobile_auth_context_for_profile(&state, device.profile_id)
        .await
        .map_err(|status| {
            (
                status,
                Json(ApiErrorResp {
                    error: "failed to read mobile access profile".into(),
                }),
            )
        })? {
        Some(mobile_auth) if mobile_auth.allows(MobileScope::WorkspaceRead) => {
            proxy_secure_request(&state, mobile_auth, payload)
                .await
                .map_err(|e| (e.status, Json(ApiErrorResp { error: e.message })))?
        }
        _ => mobile_scope_required_secure_response(MobileScope::WorkspaceRead)
            .map_err(|e| (e.status, Json(ApiErrorResp { error: e.message })))?,
    };

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
    mobile_auth: MobileAuthContext,
    mut payload: SecureRequestPayload,
) -> Result<SecureResponsePayload, SecureProxyError> {
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
        return Err(SecureProxyError::bad_request(
            "secure proxy only supports /api/* paths",
        ));
    }
    if secure_proxy_path_is_unnormalized(&path) {
        return Err(SecureProxyError::bad_request(
            "secure proxy path must be normalized",
        ));
    }
    let method = axum::http::Method::from_bytes(payload.method.as_bytes())
        .map_err(|_| SecureProxyError::bad_request("invalid http method"))?;
    if !mobile_secure_proxy_allows_request(&method, &path) {
        return desktop_auth_required_secure_response();
    }
    if !mobile_auth.allows(MobileScope::WorkspaceRead) {
        return mobile_scope_required_secure_response(MobileScope::WorkspaceRead);
    }
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

    let body = decode_body_b64(&payload.body_b64).map_err(SecureProxyError::bad_request_owned)?;
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
        .map_err(|_| SecureProxyError::bad_request("failed to build proxied request"))?;
    req.extensions_mut().insert(mobile_auth);

    let app = router(state.clone());
    let resp = app
        .oneshot(req)
        .await
        .map_err(|_| SecureProxyError::bad_gateway("failed to proxy request"))?;

    let status = resp.status().as_u16();
    let headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .filter_map(|(k, v)| Some((k.to_string(), v.to_str().ok()?.to_string())))
        .collect();
    let body_bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024 * 1024)
        .await
        .map_err(|_| SecureProxyError::bad_gateway("failed to read proxied response"))?;
    let body_b64 = base64::engine::general_purpose::STANDARD.encode(body_bytes);
    Ok(SecureResponsePayload {
        status,
        headers,
        body_b64,
    })
}

pub(super) struct SecureProxyError {
    status: StatusCode,
    message: String,
}

impl SecureProxyError {
    fn bad_request(message: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.to_string(),
        }
    }

    fn bad_request_owned(message: String) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    }

    fn bad_gateway(message: &str) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: message.to_string(),
        }
    }
}

fn mobile_secure_proxy_allows_request(method: &axum::http::Method, path: &str) -> bool {
    if *method == axum::http::Method::GET && path == "/api/health" {
        return true;
    }
    if *method == axum::http::Method::GET && path == "/api/workspaces" {
        return true;
    }
    if *method == axum::http::Method::GET {
        if let Some(workspace_id) = path.strip_prefix("/api/workspaces/") {
            return !workspace_id.is_empty() && !workspace_id.contains('/');
        }
    }
    false
}

fn secure_proxy_path_is_unnormalized(path: &str) -> bool {
    if path.contains('%') {
        return true;
    }
    let mut saw_leading = false;
    for segment in path.split('/') {
        if !saw_leading {
            saw_leading = true;
            if !segment.is_empty() {
                return true;
            }
            continue;
        }
        if segment.is_empty() || segment == "." || segment == ".." {
            return true;
        }
    }
    false
}

fn desktop_auth_required_secure_response() -> Result<SecureResponsePayload, SecureProxyError> {
    secure_error_response("desktop auth required")
}

fn mobile_scope_required_secure_response(
    scope: MobileScope,
) -> Result<SecureResponsePayload, SecureProxyError> {
    secure_error_response(scope.missing_error())
}

fn secure_error_response(message: &str) -> Result<SecureResponsePayload, SecureProxyError> {
    let body = serde_json::to_vec(&ApiErrorResp {
        error: message.to_string(),
    })
    .map_err(|_| SecureProxyError::bad_gateway("failed to encode secure response"))?;
    Ok(SecureResponsePayload {
        status: StatusCode::UNAUTHORIZED.as_u16(),
        headers: vec![(
            header::CONTENT_TYPE.as_str().to_string(),
            "application/json".to_string(),
        )],
        body_b64: base64::engine::general_purpose::STANDARD.encode(body),
    })
}
