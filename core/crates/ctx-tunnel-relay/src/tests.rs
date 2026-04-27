use super::*;
use crate::http_proxy::{extract_forward_headers, extract_ws_forward_headers};

fn test_tunnel() -> Arc<Tunnel> {
    Arc::new(Tunnel {
        inner: Mutex::new(TunnelInner {
            desktop: None,
            pending_http: HashMap::new(),
            pending_ws_open: HashMap::new(),
            ws_streams: HashMap::new(),
        }),
    })
}

#[tokio::test]
async fn dispatch_client_message_delivers_pending_http_response() {
    let tunnel = test_tunnel();
    let (tx, rx) = oneshot::channel();
    {
        let mut inner = tunnel.inner.lock().await;
        inner.pending_http.insert("req-1".to_string(), tx);
    }

    dispatch_client_message(
        &tunnel,
        ClientToRelay::HttpResponse {
            id: "req-1".to_string(),
            status: 204,
            headers: vec![("content-type".to_string(), "text/plain".to_string())],
            body_b64: BASE64.encode("ok"),
        },
    )
    .await;

    let response = rx.await.expect("http response");
    assert_eq!(response.status, 204);
    assert_eq!(response.body, b"ok");
    assert!(tunnel.inner.lock().await.pending_http.is_empty());
}

#[tokio::test]
async fn dispatch_client_message_delivers_ws_open_result_and_forwards_messages() {
    let tunnel = test_tunnel();
    let (open_tx, open_rx) = oneshot::channel();
    let (mobile_tx, mut mobile_rx) = mpsc::unbounded_channel();
    {
        let mut inner = tunnel.inner.lock().await;
        inner
            .pending_ws_open
            .insert("stream-1".to_string(), open_tx);
        inner
            .ws_streams
            .insert("stream-1".to_string(), WsStreamHandle { mobile_tx });
    }

    dispatch_client_message(
        &tunnel,
        ClientToRelay::WsOpenResult {
            id: "stream-1".to_string(),
            ok: true,
            error: None,
        },
    )
    .await;
    assert!(open_rx.await.expect("ws open result").is_ok());

    dispatch_client_message(
        &tunnel,
        ClientToRelay::WsMessage {
            id: "stream-1".to_string(),
            is_binary: false,
            data: "hello".to_string(),
        },
    )
    .await;
    let forwarded = mobile_rx.recv().await.expect("forwarded mobile message");
    assert_eq!(forwarded, Message::Text("hello".to_string()));
}

#[tokio::test]
async fn dispatch_client_message_ignores_unknown_ids_and_removes_closed_streams() {
    let tunnel = test_tunnel();
    let (mobile_tx, mut mobile_rx) = mpsc::unbounded_channel();
    {
        let mut inner = tunnel.inner.lock().await;
        inner
            .ws_streams
            .insert("stream-1".to_string(), WsStreamHandle { mobile_tx });
    }

    dispatch_client_message(
        &tunnel,
        ClientToRelay::WsMessage {
            id: "missing".to_string(),
            is_binary: false,
            data: "ignored".to_string(),
        },
    )
    .await;
    assert!(mobile_rx.try_recv().is_err());

    dispatch_client_message(
        &tunnel,
        ClientToRelay::WsClosed {
            id: "stream-1".to_string(),
            code: Some(1000),
            reason: Some("done".to_string()),
        },
    )
    .await;
    let closed = mobile_rx.recv().await.expect("close forwarded");
    assert!(matches!(closed, Message::Close(_)));
    assert!(tunnel.inner.lock().await.ws_streams.is_empty());
}

#[test]
fn relay_header_helpers_strip_hop_by_hop_headers() {
    let mut headers = HeaderMap::new();
    headers.insert(axum::http::header::HOST, "example.test".parse().unwrap());
    headers.insert(axum::http::header::CONNECTION, "upgrade".parse().unwrap());
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer secret".parse().unwrap(),
    );
    headers.insert("x-custom", "value".parse().unwrap());

    let forwarded = extract_forward_headers(&headers);
    assert!(forwarded.contains(&(String::from("authorization"), String::from("Bearer secret"))));
    assert!(forwarded.contains(&(String::from("x-custom"), String::from("value"))));
    assert!(!forwarded
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("host")));
    assert!(!forwarded
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("connection")));

    let ws_forwarded = extract_ws_forward_headers(&headers);
    assert_eq!(
        ws_forwarded,
        vec![(String::from("authorization"), String::from("Bearer secret"))]
    );
}

#[test]
fn derive_secret_is_deterministic() {
    let first = derive_secret(b"master-secret", "tunnel-1").unwrap();
    let second = derive_secret(b"master-secret", "tunnel-1").unwrap();
    let third = derive_secret(b"master-secret", "tunnel-2").unwrap();
    assert_eq!(first, second);
    assert_ne!(first, third);
}

#[test]
fn parse_master_secret_rejects_empty_values() {
    assert!(parse_master_secret("").is_err());
    assert!(parse_master_secret("   ").is_err());
}
