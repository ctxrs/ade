use super::*;
use ctx_transport_runtime::{
    mobile_secure_proxy_allows_request, secure_proxy_path_is_unnormalized,
};
use tower::util::ServiceExt;

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
    pub(super) fn into_api_error(self) -> (StatusCode, Json<ApiErrorResp>) {
        (
            self.status,
            Json(ApiErrorResp {
                error: self.message,
            }),
        )
    }

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

fn desktop_auth_required_secure_response() -> Result<SecureResponsePayload, SecureProxyError> {
    secure_error_response("desktop auth required")
}

pub(super) fn mobile_scope_required_secure_response(
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
