use super::*;
use ctx_transport_runtime::{
    mobile_secure_proxy_allows_request, secure_proxy_path_is_unnormalized,
};
use errors::desktop_auth_required_secure_response;
pub(super) use errors::{mobile_scope_required_secure_response, SecureProxyError};

use crate::api::router::RouteState;
use ctx_daemon::daemon::{CoreHandle, WorkspacesHandle};

#[path = "secure_proxy/errors.rs"]
mod errors;

#[derive(Clone)]
pub(in crate::api) struct SecureProxyRouterState {
    core: CoreHandle,
    workspaces: WorkspacesHandle,
}

impl axum::extract::FromRef<RouteState> for SecureProxyRouterState {
    fn from_ref(state: &RouteState) -> Self {
        Self {
            core: state.handles.core.clone(),
            workspaces: state.handles.workspaces.clone(),
        }
    }
}

pub(super) async fn proxy_secure_request(
    router_state: &SecureProxyRouterState,
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

    let _body = decode_body_b64(&payload.body_b64).map_err(SecureProxyError::bad_request_owned)?;
    let mut headers = HeaderMap::new();
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
        headers.insert(header_name, header_value);
    }

    let resp =
        dispatch_scoped_secure_proxy_request(router_state, method, &uri, headers, mobile_auth)
            .await;

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

async fn dispatch_scoped_secure_proxy_request(
    state: &SecureProxyRouterState,
    method: axum::http::Method,
    uri: &str,
    headers: HeaderMap,
    _mobile_auth: MobileAuthContext,
) -> Response {
    let path = uri.split_once('?').map(|(path, _)| path).unwrap_or(uri);
    if method != axum::http::Method::GET {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    if path == "/api/health" {
        return health(State(state.core.clone()), headers)
            .await
            .into_response();
    }
    if path == "/api/workspaces" {
        return list_workspaces(State(state.workspaces.clone()))
            .await
            .into_response();
    }
    if let Some(workspace_id) = path.strip_prefix("/api/workspaces/") {
        return get_workspace(
            State(state.workspaces.clone()),
            Path(workspace_id.to_string()),
        )
        .await
        .into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}
