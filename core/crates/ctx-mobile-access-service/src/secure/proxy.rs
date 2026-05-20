use base64::Engine;

use crate::{MobileAccessServiceError, MobileAuthContext, MobileScope, MobileSecureProxyPayload};

const DESKTOP_AUTH_REQUIRED: &str = "desktop auth required";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobileSecureProxyAdmission {
    Admitted(MobileSecureProxyAdmittedRequest),
    Denied(MobileSecureProxyDenyReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileSecureProxyAdmittedRequest {
    pub uri: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileSecureProxyDenyReason {
    message: &'static str,
}

impl MobileSecureProxyDenyReason {
    pub fn message(&self) -> &'static str {
        self.message
    }
}

pub fn prepare_mobile_secure_proxy_request(
    mobile_auth: Option<MobileAuthContext>,
    mut payload: MobileSecureProxyPayload,
) -> Result<MobileSecureProxyAdmission, MobileAccessServiceError> {
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
        return Err(MobileAccessServiceError::bad_request(
            "secure proxy only supports /api/* paths",
        ));
    }
    if secure_proxy_path_is_unnormalized(&path) {
        return Err(MobileAccessServiceError::bad_request(
            "secure proxy path must be normalized",
        ));
    }

    let method = parse_proxy_method(&payload.method)?;
    if !mobile_secure_proxy_allows_request(method, &path) {
        return Ok(MobileSecureProxyAdmission::Denied(
            MobileSecureProxyDenyReason {
                message: DESKTOP_AUTH_REQUIRED,
            },
        ));
    }

    let Some(mobile_auth) = mobile_auth else {
        return Ok(MobileSecureProxyAdmission::Denied(
            MobileSecureProxyDenyReason {
                message: MobileScope::WorkspaceRead.missing_error(),
            },
        ));
    };
    if !mobile_auth.allows(MobileScope::WorkspaceRead) {
        return Ok(MobileSecureProxyAdmission::Denied(
            MobileSecureProxyDenyReason {
                message: MobileScope::WorkspaceRead.missing_error(),
            },
        ));
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

    let _body =
        decode_proxy_body_b64(&payload.body_b64).map_err(MobileAccessServiceError::bad_request)?;

    Ok(MobileSecureProxyAdmission::Admitted(
        MobileSecureProxyAdmittedRequest {
            uri,
            headers: payload.headers,
        },
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MobileSecureProxyMethod {
    Get,
    Other,
}

fn parse_proxy_method(method: &str) -> Result<MobileSecureProxyMethod, MobileAccessServiceError> {
    if method == "GET" {
        return Ok(MobileSecureProxyMethod::Get);
    }
    if !method.is_empty() && method.bytes().all(is_http_token_byte) {
        return Ok(MobileSecureProxyMethod::Other);
    }
    Err(MobileAccessServiceError::bad_request("invalid http method"))
}

fn is_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

fn mobile_secure_proxy_allows_request(method: MobileSecureProxyMethod, path: &str) -> bool {
    if method != MobileSecureProxyMethod::Get {
        return false;
    }
    if path == "/api/health" || path == "/api/workspaces" {
        return true;
    }
    if let Some(workspace_id) = path.strip_prefix("/api/workspaces/") {
        return !workspace_id.is_empty() && !workspace_id.contains('/');
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MobileScopeSet, MobileSecureProxyPayload};
    use ctx_core::ids::ConnectionProfileId;

    fn auth() -> MobileAuthContext {
        MobileAuthContext::new_for_test(
            ConnectionProfileId(uuid::Uuid::nil()),
            MobileScopeSet::managed_default(),
        )
    }

    fn payload(method: &str, path: &str) -> MobileSecureProxyPayload {
        MobileSecureProxyPayload {
            method: method.to_string(),
            path: path.to_string(),
            query: None,
            headers: Vec::new(),
            body_b64: String::new(),
        }
    }

    #[test]
    fn mobile_secure_proxy_allows_only_workspace_read_routes() {
        assert!(matches!(
            prepare_mobile_secure_proxy_request(Some(auth()), payload("GET", "/api/health")),
            Ok(MobileSecureProxyAdmission::Admitted(_))
        ));
        assert!(matches!(
            prepare_mobile_secure_proxy_request(Some(auth()), payload("GET", "/api/workspaces")),
            Ok(MobileSecureProxyAdmission::Admitted(_))
        ));
        assert!(matches!(
            prepare_mobile_secure_proxy_request(
                Some(auth()),
                payload(
                    "GET",
                    "/api/workspaces/00000000-0000-0000-0000-000000000001"
                )
            ),
            Ok(MobileSecureProxyAdmission::Admitted(_))
        ));

        assert!(matches!(
            prepare_mobile_secure_proxy_request(Some(auth()), payload("POST", "/api/workspaces")),
            Ok(MobileSecureProxyAdmission::Denied(_))
        ));
        assert!(matches!(
            prepare_mobile_secure_proxy_request(Some(auth()), payload("get", "/api/workspaces")),
            Ok(MobileSecureProxyAdmission::Denied(_))
        ));
        assert!(matches!(
            prepare_mobile_secure_proxy_request(
                Some(auth()),
                payload(
                    "GET",
                    "/api/workspaces/00000000-0000-0000-0000-000000000001/tasks"
                )
            ),
            Ok(MobileSecureProxyAdmission::Denied(_))
        ));
        assert!(matches!(
            prepare_mobile_secure_proxy_request(Some(auth()), payload("GET", "/api/providers")),
            Ok(MobileSecureProxyAdmission::Denied(_))
        ));
    }

    #[test]
    fn secure_proxy_rejects_unnormalized_paths() {
        assert!(prepare_mobile_secure_proxy_request(
            Some(auth()),
            payload("GET", "api/workspaces")
        )
        .is_err());
        assert!(prepare_mobile_secure_proxy_request(
            Some(auth()),
            payload("GET", "/api//workspaces")
        )
        .is_err());
        assert!(prepare_mobile_secure_proxy_request(
            Some(auth()),
            payload("GET", "/api/./workspaces")
        )
        .is_err());
        assert!(prepare_mobile_secure_proxy_request(
            Some(auth()),
            payload("GET", "/api/../providers")
        )
        .is_err());
        assert!(prepare_mobile_secure_proxy_request(
            Some(auth()),
            payload("GET", "/api/%2e%2e/providers")
        )
        .is_err());
    }

    #[test]
    fn path_query_is_split_before_admission() {
        let mut request = payload(
            "GET",
            "/api/workspaces/00000000-0000-0000-0000-000000000001?include=summary",
        );
        request.query = None;
        let result =
            prepare_mobile_secure_proxy_request(Some(auth()), request).expect("proxy admission");

        let MobileSecureProxyAdmission::Admitted(admitted) = result else {
            panic!("expected admitted request");
        };
        assert_eq!(
            admitted.uri,
            "/api/workspaces/00000000-0000-0000-0000-000000000001?include=summary"
        );
    }

    #[test]
    fn proxy_body_decoding_accepts_empty_padded_and_urlsafe_bodies() {
        let mut request = payload("GET", "/api/health");
        request.body_b64 = String::new();
        prepare_mobile_secure_proxy_request(Some(auth()), request).expect("empty body");

        let mut request = payload("GET", "/api/health");
        request.body_b64 = base64::engine::general_purpose::STANDARD.encode(b"hello");
        prepare_mobile_secure_proxy_request(Some(auth()), request).expect("padded body");

        let mut request = payload("GET", "/api/health");
        request.body_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"hello?");
        prepare_mobile_secure_proxy_request(Some(auth()), request).expect("urlsafe body");
    }

    #[test]
    fn proxy_body_decoding_rejects_invalid_body_before_dispatch() {
        let mut request = payload("GET", "/api/health");
        request.body_b64 = "***".to_string();
        let error = prepare_mobile_secure_proxy_request(Some(auth()), request).unwrap_err();
        assert_eq!(error.message(), "invalid base64 body");
    }
}
