use base64::Engine;
use ctx_core::ids::WorkspaceId;
use ctx_transport_runtime::{
    mobile_secure_proxy_allows_request, secure_proxy_path_is_unnormalized,
};
use http::{header, Method, StatusCode};
use serde::Serialize;

use super::{
    MobileAccessRouteError, MobileAccessRouteErrorKind, MobileAuthContext, MobileScope,
    MobileSecureProxyPayload, MobileSecureProxyResponsePayload,
};
use crate::daemon::CoreHandle;

const JSON_CONTENT_TYPE: &str = "application/json";
const DESKTOP_AUTH_REQUIRED: &str = "desktop auth required";

impl CoreHandle {
    pub async fn proxy_mobile_secure_request_for_route(
        &self,
        mobile_auth: Option<MobileAuthContext>,
        mut payload: MobileSecureProxyPayload,
        package_version: &'static str,
    ) -> Result<MobileSecureProxyResponsePayload, MobileAccessRouteError> {
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
            return Err(MobileAccessRouteError::bad_request(
                "secure proxy only supports /api/* paths",
            ));
        }
        if secure_proxy_path_is_unnormalized(&path) {
            return Err(MobileAccessRouteError::bad_request(
                "secure proxy path must be normalized",
            ));
        }

        let method = Method::from_bytes(payload.method.as_bytes())
            .map_err(|_| MobileAccessRouteError::bad_request("invalid http method"))?;
        if !mobile_secure_proxy_allows_request(&method, &path) {
            return secure_error_response(DESKTOP_AUTH_REQUIRED);
        }

        let Some(mobile_auth) = mobile_auth else {
            return secure_error_response(MobileScope::WorkspaceRead.missing_error());
        };
        if !mobile_auth.allows(MobileScope::WorkspaceRead) {
            return secure_error_response(MobileScope::WorkspaceRead.missing_error());
        }

        let mut uri = path.clone();
        if let Some(query) = payload
            .query
            .as_ref()
            .map(|q| q.trim())
            .filter(|q| !q.is_empty())
        {
            uri.push('?');
            uri.push_str(query.trim_start_matches('?'));
        }

        let _body = decode_proxy_body_b64(&payload.body_b64)
            .map_err(MobileAccessRouteError::bad_request)?;

        dispatch_scoped_secure_proxy_request(self, &method, &uri, &payload.headers, package_version)
            .await
    }
}

async fn dispatch_scoped_secure_proxy_request(
    core: &CoreHandle,
    method: &Method,
    uri: &str,
    headers: &[(String, String)],
    package_version: &'static str,
) -> Result<MobileSecureProxyResponsePayload, MobileAccessRouteError> {
    let path = uri.split_once('?').map(|(path, _)| path).unwrap_or(uri);
    if method != Method::GET {
        return Ok(empty_response(StatusCode::METHOD_NOT_ALLOWED));
    }
    if path == "/api/health" {
        let include_sensitive = health_request_is_authorized(core, headers);
        let Ok(snapshot) = core.health_snapshot(package_version, include_sensitive) else {
            return Ok(empty_response(StatusCode::INTERNAL_SERVER_ERROR));
        };
        return json_response(StatusCode::OK, &snapshot);
    }
    if path == "/api/workspaces" {
        let Ok(workspaces) = core.state.global_store().list_workspaces().await else {
            return Ok(empty_response(StatusCode::INTERNAL_SERVER_ERROR));
        };
        return json_response(StatusCode::OK, &workspaces);
    }
    if let Some(workspace_id) = path.strip_prefix("/api/workspaces/") {
        let Ok(workspace_uuid) = uuid::Uuid::parse_str(workspace_id) else {
            return Ok(empty_response(StatusCode::BAD_REQUEST));
        };
        let workspace_id = WorkspaceId(workspace_uuid);
        let Ok(workspace) = core.state.global_store().get_workspace(workspace_id).await else {
            return Ok(empty_response(StatusCode::INTERNAL_SERVER_ERROR));
        };
        if let Some(workspace) = workspace {
            core.state
                .telemetry
                .telemetry
                .emit(ctx_observability::telemetry::TelemetryEvent::workspace_opened())
                .await;
            return json_response(StatusCode::OK, &workspace);
        }
        return Ok(empty_response(StatusCode::NOT_FOUND));
    }
    Ok(empty_response(StatusCode::NOT_FOUND))
}

fn health_request_is_authorized(core: &CoreHandle, headers: &[(String, String)]) -> bool {
    let Some(expected) = core.auth_token() else {
        return true;
    };
    headers.iter().any(|(name, value)| {
        if name.eq_ignore_ascii_case("host") || name.eq_ignore_ascii_case("content-length") {
            return false;
        }
        let Ok(header_name) = header::HeaderName::from_bytes(name.as_bytes()) else {
            return false;
        };
        let Ok(header_value) = header::HeaderValue::from_str(value) else {
            return false;
        };
        if header_name != header::AUTHORIZATION {
            return false;
        }
        header_value
            .to_str()
            .ok()
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|value| value == expected)
    })
}

fn secure_error_response(
    message: &str,
) -> Result<MobileSecureProxyResponsePayload, MobileAccessRouteError> {
    json_response(
        StatusCode::UNAUTHORIZED,
        &serde_json::json!({ "error": message }),
    )
}

fn json_response<T: Serialize>(
    status: StatusCode,
    value: &T,
) -> Result<MobileSecureProxyResponsePayload, MobileAccessRouteError> {
    let body = serde_json::to_vec(value).map_err(|_| {
        MobileAccessRouteError::new(
            MobileAccessRouteErrorKind::BadGateway,
            "failed to encode secure response",
        )
    })?;
    Ok(MobileSecureProxyResponsePayload {
        status: status.as_u16(),
        headers: vec![(
            header::CONTENT_TYPE.as_str().to_string(),
            JSON_CONTENT_TYPE.to_string(),
        )],
        body_b64: base64::engine::general_purpose::STANDARD.encode(body),
    })
}

fn empty_response(status: StatusCode) -> MobileSecureProxyResponsePayload {
    MobileSecureProxyResponsePayload {
        status: status.as_u16(),
        headers: Vec::new(),
        body_b64: String::new(),
    }
}

fn decode_proxy_body_b64(value: &str) -> Result<Vec<u8>, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let mut normalized = trimmed.replace('-', "+").replace('_', "/");
    while !normalized.len().is_multiple_of(4) {
        normalized.push('=');
    }
    base64::engine::general_purpose::STANDARD
        .decode(normalized.as_bytes())
        .map_err(|_| "invalid base64 body".to_string())
}
