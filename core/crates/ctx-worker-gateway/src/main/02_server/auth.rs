use super::*;

pub(super) async fn auth_middleware(
    State(state): State<AppState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let path = req.uri().path();
    let method = req.method().as_str();
    if gateway_request_is_authorized(
        state.auth_token.as_deref(),
        method,
        path,
        req.uri().query(),
        req.headers(),
    ) {
        return next.run(req).await;
    }

    (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
}

fn gateway_request_is_authorized(
    expected: Option<&str>,
    method: &str,
    path: &str,
    query: Option<&str>,
    headers: &axum::http::HeaderMap,
) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    if path == "/health" {
        return true;
    }
    if method == "GET"
        && !path.contains("/bootstrap")
        && path.starts_with("/workers/")
        && !path.contains("/terminals/")
        && !path.ends_with("/export")
    {
        return true;
    }

    let header_token = headers
        .get("x-ctx-gateway-token")
        .and_then(|value| value.to_str().ok());
    if header_token == Some(expected) {
        return true;
    }

    (path == "/shim" || path.contains("/bootstrap"))
        && query.is_some_and(|query| {
            query.split('&').any(
                |part| matches!(part.split_once('='), Some(("token", value)) if value == expected),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with_token(token: &str) -> axum::http::HeaderMap {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-ctx-gateway-token",
            axum::http::HeaderValue::from_str(token).expect("header token"),
        );
        headers
    }

    #[test]
    fn gateway_auth_allows_public_and_read_only_routes() {
        assert!(gateway_request_is_authorized(
            Some("secret"),
            "GET",
            "/health",
            None,
            &axum::http::HeaderMap::new(),
        ));
        assert!(gateway_request_is_authorized(
            Some("secret"),
            "GET",
            "/workers/worker-1",
            None,
            &axum::http::HeaderMap::new(),
        ));
        assert!(!gateway_request_is_authorized(
            Some("secret"),
            "GET",
            "/workers/worker-1/export",
            None,
            &axum::http::HeaderMap::new(),
        ));
        assert!(!gateway_request_is_authorized(
            Some("secret"),
            "GET",
            "/workers/worker-1/terminals/terminal-1/daemon",
            None,
            &axum::http::HeaderMap::new(),
        ));
    }

    #[test]
    fn gateway_auth_accepts_header_token_for_private_routes() {
        assert!(gateway_request_is_authorized(
            Some("secret"),
            "POST",
            "/workers",
            None,
            &headers_with_token("secret"),
        ));
        assert!(!gateway_request_is_authorized(
            Some("secret"),
            "POST",
            "/workers",
            None,
            &headers_with_token("wrong"),
        ));
    }

    #[test]
    fn gateway_auth_accepts_query_token_for_bootstrap_and_shim() {
        assert!(gateway_request_is_authorized(
            Some("secret"),
            "GET",
            "/workers/worker-1/bootstrap",
            Some("token=secret"),
            &axum::http::HeaderMap::new(),
        ));
        assert!(gateway_request_is_authorized(
            Some("secret"),
            "GET",
            "/shim",
            Some("foo=bar&token=secret"),
            &axum::http::HeaderMap::new(),
        ));
        assert!(!gateway_request_is_authorized(
            Some("secret"),
            "GET",
            "/workers/worker-1/bootstrap",
            Some("token=wrong"),
            &axum::http::HeaderMap::new(),
        ));
    }

    #[test]
    fn gateway_auth_allows_everything_when_token_is_unset() {
        assert!(gateway_request_is_authorized(
            None,
            "POST",
            "/workers",
            None,
            &axum::http::HeaderMap::new(),
        ));
    }
}
