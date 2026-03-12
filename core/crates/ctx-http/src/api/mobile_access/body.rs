use super::ApiErrorResp;
use axum::body::Bytes;
use axum::Json;
use base64::Engine;
use http::StatusCode;

pub(super) fn parse_json_body<T: serde::de::DeserializeOwned>(
    body: Bytes,
) -> Result<T, (StatusCode, Json<ApiErrorResp>)> {
    if body.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "missing request body".into(),
            }),
        ));
    }
    serde_json::from_slice(&body).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid json body".into(),
            }),
        )
    })
}

pub(super) fn decode_body_b64(value: &str) -> Result<Vec<u8>, String> {
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
