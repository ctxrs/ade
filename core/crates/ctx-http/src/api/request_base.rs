#[cfg(test)]
use axum::http::HeaderValue;
use axum::http::{header, HeaderMap};
use url::Url;

fn header_first_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = headers.get(name)?.to_str().ok()?;
    Some(value.split(',').next()?.trim().to_string())
}

fn parse_forwarded_header(value: &str) -> (Option<String>, Option<String>) {
    let mut proto = None;
    let mut host = None;
    let first = value.split(',').next().unwrap_or(value);
    for part in first.split(';') {
        let part = part.trim();
        if let Some(raw) = part.strip_prefix("proto=") {
            let clean = raw.trim().trim_matches('"').trim_matches('\'');
            if !clean.is_empty() {
                proto = Some(clean.to_string());
            }
        } else if let Some(raw) = part.strip_prefix("host=") {
            let clean = raw.trim().trim_matches('"').trim_matches('\'');
            if !clean.is_empty() {
                host = Some(clean.to_string());
            }
        }
    }
    (proto, host)
}

pub(super) fn resolve_request_base_url(
    headers: &HeaderMap,
    fallback: &str,
    public_base_url: Option<&str>,
) -> Option<String> {
    if let Some(public_base_url) = public_base_url {
        let trimmed = public_base_url.trim().trim_end_matches('/');
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    let fallback = fallback.trim_end_matches('/');
    let fallback_url = Url::parse(fallback).ok();
    let fallback_base = fallback_url.as_ref().and_then(|url| {
        let host = url.host_str()?;
        let host = match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        };
        if is_safe_request_base_host(&host) && matches!(url.scheme(), "http" | "https") {
            Some(format!("{}://{}", url.scheme(), host.trim_end_matches('/')))
        } else {
            None
        }
    });

    let (forwarded_proto, forwarded_host) = headers
        .get(header::FORWARDED)
        .and_then(|value| value.to_str().ok())
        .map(parse_forwarded_header)
        .unwrap_or((None, None));

    let proto = forwarded_proto
        .or_else(|| header_first_value(headers, "x-forwarded-proto"))
        .unwrap_or_else(|| {
            fallback_url
                .as_ref()
                .map(|url| url.scheme().to_string())
                .unwrap_or_else(|| "http".to_string())
        })
        .trim()
        .trim_end_matches(':')
        .to_ascii_lowercase();
    let host = forwarded_host
        .or_else(|| header_first_value(headers, "x-forwarded-host"))
        .or_else(|| header_first_value(headers, header::HOST.as_str()));

    match host {
        Some(host)
            if is_safe_request_base_host(&host) && matches!(proto.as_str(), "http" | "https") =>
        {
            Some(format!("{}://{}", proto, host.trim_end_matches('/')))
        }
        Some(_) => None,
        None => fallback_base,
    }
}

pub(super) fn public_route_url(base_url: &str, route_path: &str) -> Option<String> {
    let mut base = Url::parse(base_url).ok()?;
    let normalized_base_path = match base.path().trim_end_matches('/') {
        "" => "/".to_string(),
        path => format!("{path}/"),
    };
    base.set_path(&normalized_base_path);
    base.join(route_path.trim_start_matches('/'))
        .ok()
        .map(|url| url.to_string())
}

pub(super) fn public_websocket_url(base_url: &str, route_path: &str) -> Option<String> {
    let mut url =
        public_route_url(base_url, route_path).and_then(|joined| Url::parse(&joined).ok())?;
    match url.scheme() {
        "http" => {
            url.set_scheme("ws").ok()?;
        }
        "https" => {
            url.set_scheme("wss").ok()?;
        }
        _ => return None,
    }
    Some(url.to_string())
}

pub(super) fn is_loopback_host(host: &str) -> bool {
    let normalized = host.trim().trim_matches('[').trim_matches(']');
    normalized.eq_ignore_ascii_case("localhost")
        || normalized.eq_ignore_ascii_case("tauri.localhost")
        || normalized == "127.0.0.1"
        || normalized == "::1"
}

fn is_safe_request_base_host(host: &str) -> bool {
    let trimmed = host.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return false;
    }
    let parsed = match Url::parse(&format!("http://{trimmed}")) {
        Ok(url) => url,
        Err(_) => return false,
    };
    parsed.host_str().map(is_loopback_host).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_request_base_url_accepts_loopback_host_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:4455"));
        assert_eq!(
            resolve_request_base_url(&headers, "http://127.0.0.1:4321", None),
            Some("http://127.0.0.1:4455".to_string())
        );
    }

    #[test]
    fn resolve_request_base_url_rejects_non_loopback_host_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("evil.example"));
        assert_eq!(
            resolve_request_base_url(&headers, "http://127.0.0.1:4321", None),
            None
        );
    }

    #[test]
    fn resolve_request_base_url_rejects_non_http_forwarded_proto() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::FORWARDED,
            HeaderValue::from_static("proto=javascript;host=127.0.0.1:4455"),
        );
        assert_eq!(
            resolve_request_base_url(&headers, "http://127.0.0.1:4321", None),
            None
        );
    }

    #[test]
    fn resolve_request_base_url_accepts_forwarded_loopback_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::FORWARDED,
            HeaderValue::from_static("proto=https;host=tauri.localhost:3000"),
        );
        assert_eq!(
            resolve_request_base_url(&headers, "http://127.0.0.1:4321", None),
            Some("https://tauri.localhost:3000".to_string())
        );
    }

    #[test]
    fn resolve_request_base_url_uses_loopback_fallback_without_request_host() {
        let headers = HeaderMap::new();
        assert_eq!(
            resolve_request_base_url(&headers, "http://127.0.0.1:4321", None),
            Some("http://127.0.0.1:4321".to_string())
        );
    }

    #[test]
    fn resolve_request_base_url_prefers_configured_public_base_url() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:4455"));
        headers.insert(
            header::FORWARDED,
            HeaderValue::from_static("proto=https;host=proxy.example"),
        );
        assert_eq!(
            resolve_request_base_url(
                &headers,
                "http://127.0.0.1:4321",
                Some("https://proxy.example/ctx"),
            ),
            Some("https://proxy.example/ctx".to_string())
        );
    }

    #[test]
    fn public_route_url_preserves_path_prefix_and_query() {
        assert_eq!(
            public_route_url(
                "https://proxy.example/ctx",
                "/sessions/web/sess-1/view?token=stream-token",
            ),
            Some(
                "https://proxy.example/ctx/sessions/web/sess-1/view?token=stream-token".to_string(),
            )
        );
    }

    #[test]
    fn public_websocket_url_preserves_path_prefix_and_query() {
        assert_eq!(
            public_websocket_url(
                "https://proxy.example/ctx",
                "/sessions/web/sess-1/signal?token=signal-token",
            ),
            Some(
                "wss://proxy.example/ctx/sessions/web/sess-1/signal?token=signal-token".to_string(),
            )
        );
    }
}
