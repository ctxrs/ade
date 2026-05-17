use super::*;

pub(in crate::api::mobile_access) struct SecureProxyError {
    status: StatusCode,
    message: String,
}

impl SecureProxyError {
    pub(in crate::api::mobile_access) fn into_api_error(self) -> (StatusCode, Json<ApiErrorResp>) {
        (
            self.status,
            Json(ApiErrorResp {
                error: self.message,
            }),
        )
    }

    pub(super) fn bad_request(message: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.to_string(),
        }
    }

    pub(super) fn bad_request_owned(message: String) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    }

    pub(super) fn bad_gateway(message: &str) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: message.to_string(),
        }
    }
}

pub(super) fn desktop_auth_required_secure_response(
) -> Result<MobileSecureProxyResponsePayload, SecureProxyError> {
    secure_error_response("desktop auth required")
}

pub(in crate::api::mobile_access) fn mobile_scope_required_secure_response(
    scope: MobileScope,
) -> Result<MobileSecureProxyResponsePayload, SecureProxyError> {
    secure_error_response(scope.missing_error())
}

fn secure_error_response(
    message: &str,
) -> Result<MobileSecureProxyResponsePayload, SecureProxyError> {
    let body = serde_json::to_vec(&ApiErrorResp {
        error: message.to_string(),
    })
    .map_err(|_| SecureProxyError::bad_gateway("failed to encode secure response"))?;
    Ok(MobileSecureProxyResponsePayload {
        status: StatusCode::UNAUTHORIZED.as_u16(),
        headers: vec![(
            header::CONTENT_TYPE.as_str().to_string(),
            "application/json".to_string(),
        )],
        body_b64: base64::engine::general_purpose::STANDARD.encode(body),
    })
}
