use super::*;

pub(super) async fn proxy_get(
    State(state): State<RelayState>,
    Path((tunnel_id, path)): Path<(String, String)>,
    req: Request<axum::body::Body>,
) -> Response {
    let (mut parts, body) = req.into_parts();
    let is_ws = is_websocket_upgrade(&parts.headers);
    if is_ws {
        match axum::extract::ws::WebSocketUpgrade::from_request_parts(&mut parts, &state).await {
            Ok(ws) => {
                let headers = parts.headers.clone();
                let uri = parts.uri.clone();
                return ws
                    .on_upgrade(move |socket| async move {
                        if let Err(err) = handle_mobile_ws(
                            state,
                            tunnel_id,
                            path,
                            uri.query().map(str::to_string),
                            headers,
                            socket,
                        )
                        .await
                        {
                            warn!("mobile ws error: {err:#}");
                        }
                    })
                    .into_response();
            }
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        }
    }

    let req = Request::from_parts(parts, body);
    proxy_http_inner(state, tunnel_id, path, req).await
}

pub(super) async fn proxy_http(
    State(state): State<RelayState>,
    Path((tunnel_id, path)): Path<(String, String)>,
    uri: Uri,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let uri_path = format!("/{}", path.trim_start_matches('/'));
    proxy_http_message(
        state,
        tunnel_id,
        method,
        uri_path,
        uri.query().map(str::to_string),
        headers,
        body,
    )
    .await
}

pub(super) async fn proxy_http_inner(
    state: RelayState,
    tunnel_id: String,
    path: String,
    req: Request<axum::body::Body>,
) -> Response {
    let (parts, body) = req.into_parts();
    let method = parts.method;
    let headers = parts.headers;
    let query = parts.uri.query().map(str::to_string);
    let bytes = match axum::body::to_bytes(body, 16 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let uri = format!("/{}", path.trim_start_matches('/'));
    proxy_http_message(state, tunnel_id, method, uri, query, headers, bytes).await
}

pub(super) async fn proxy_http_message(
    state: RelayState,
    tunnel_id: String,
    method: Method,
    path: String,
    query: Option<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let tunnel = get_or_create_tunnel(&state, &tunnel_id).await;

    let desktop = {
        let inner = tunnel.inner.lock().await;
        inner.desktop.clone()
    };
    let Some(desktop) = desktop else {
        return (StatusCode::BAD_GATEWAY, "tunnel not connected").into_response();
    };

    let request_id = Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel::<HttpResponse>();
    {
        let mut inner = tunnel.inner.lock().await;
        inner.pending_http.insert(request_id.clone(), tx);
    }

    let forward_headers = extract_ws_forward_headers(&headers);
    let mut full_path = path;
    if let Some(q) = query {
        if !q.is_empty() {
            full_path.push('?');
            full_path.push_str(&q);
        }
    }

    let msg = RelayToClient::HttpRequest {
        id: request_id.clone(),
        method: method.to_string(),
        path: full_path,
        headers: forward_headers,
        body_b64: BASE64.encode(body),
    };
    if desktop.tx.send(msg).is_err() {
        return (StatusCode::BAD_GATEWAY, "tunnel disconnected").into_response();
    }

    match tokio::time::timeout(Duration::from_secs(30), rx).await {
        Ok(Ok(resp)) => build_http_response(resp),
        Ok(Err(_)) => (StatusCode::BAD_GATEWAY, "tunnel response dropped").into_response(),
        Err(_) => (StatusCode::GATEWAY_TIMEOUT, "tunnel request timed out").into_response(),
    }
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    let Some(upgrade) = headers.get(axum::http::header::UPGRADE) else {
        return false;
    };
    let Ok(upgrade) = upgrade.to_str() else {
        return false;
    };
    upgrade.eq_ignore_ascii_case("websocket")
}

fn extract_forward_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    const HOP_BY_HOP: &[axum::http::header::HeaderName] = &[
        axum::http::header::CONNECTION,
        axum::http::header::UPGRADE,
        axum::http::header::PROXY_AUTHENTICATE,
        axum::http::header::PROXY_AUTHORIZATION,
        axum::http::header::TE,
        axum::http::header::TRAILER,
        axum::http::header::TRANSFER_ENCODING,
        axum::http::header::HOST,
    ];

    headers
        .iter()
        .filter(|(k, _)| !HOP_BY_HOP.contains(k))
        .filter_map(|(k, v)| Some((k.to_string(), v.to_str().ok()?.to_string())))
        .collect()
}

fn extract_ws_forward_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(k, _)| k.as_str().eq_ignore_ascii_case("authorization"))
        .filter_map(|(k, v)| Some((k.to_string(), v.to_str().ok()?.to_string())))
        .collect()
}

fn build_http_response(resp: HttpResponse) -> Response {
    let status = StatusCode::from_u16(resp.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut builder = Response::builder().status(status);
    if let Some(headers) = builder.headers_mut() {
        for (k, v) in resp.headers {
            if let (Ok(name), Ok(value)) = (
                axum::http::header::HeaderName::from_bytes(k.as_bytes()),
                axum::http::HeaderValue::from_str(&v),
            ) {
                headers.insert(name, value);
            }
        }
    } else {
        tracing::warn!(
            "response builder headers unavailable; returning response without forwarded headers"
        );
    }
    builder
        .body(axum::body::Body::from(resp.body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
