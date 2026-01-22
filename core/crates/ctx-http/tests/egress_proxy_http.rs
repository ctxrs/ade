mod common;

use axum::body::Body;
use axum::http::{header, Method, StatusCode};
use axum::routing::get;
use axum::Router;
use base64::Engine;
use ctx_core::ids::SessionId;
use ctx_http::egress_proxy::proxy_url_for_daemon;
use ctx_http::settings::{NetworkProfile, NetworkSettings, Settings};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{timeout, Duration};
use tower::ServiceExt;
use url::Url;
use uuid::Uuid;

use common::{build_state, fake_providers, setup_store, spawn_http_server};

#[tokio::test]
async fn proxy_requires_auth() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = setup_store(data_dir.path()).await;
    let state = build_state(
        data_dir.path(),
        stores,
        fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state);
    let proxy_server = spawn_http_server(app).await;

    let target_app = Router::new().route("/", get(|| async { "ok" }));
    let target_server = spawn_http_server(target_app).await;

    let proxy = reqwest::Proxy::http(&proxy_server.base_url).unwrap();
    let client = reqwest::Client::builder().proxy(proxy).build().unwrap();
    let res = client
        .get(format!("{}/", target_server.base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(
        res.status(),
        reqwest::StatusCode::PROXY_AUTHENTICATION_REQUIRED
    );
}

#[tokio::test]
async fn proxy_allows_full_profile() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = setup_store(data_dir.path()).await;
    let state = build_state(
        data_dir.path(),
        stores,
        fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());
    let proxy_server = spawn_http_server(app).await;

    let target_app = Router::new().route("/", get(|| async { "ok" }));
    let target_server = spawn_http_server(target_app).await;

    let session_id = SessionId(Uuid::new_v4());
    let token = state.proxy_token_for_session(session_id).await;
    let proxy_url = proxy_url_for_daemon(&proxy_server.base_url, &token).unwrap();

    let proxy = reqwest::Proxy::http(&proxy_url).unwrap();
    let client = reqwest::Client::builder().proxy(proxy).build().unwrap();
    let res = client
        .get(format!("{}/", target_server.base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let body = res.text().await.unwrap();
    assert_eq!(body, "ok");
}

#[tokio::test]
async fn proxy_blocks_none_profile() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = setup_store(data_dir.path()).await;
    let state = build_state(
        data_dir.path(),
        stores,
        fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());
    let proxy_server = spawn_http_server(app).await;

    let settings = Settings {
        network: Some(NetworkSettings {
            profile: NetworkProfile::None,
            mcp_bypass: true,
        }),
        ..Settings::default()
    };
    ctx_http::settings::save_settings(data_dir.path(), &settings)
        .await
        .unwrap();

    let target_app = Router::new().route("/", get(|| async { "ok" }));
    let target_server = spawn_http_server(target_app).await;

    let session_id = SessionId(Uuid::new_v4());
    let token = state.proxy_token_for_session(session_id).await;
    let proxy_url = proxy_url_for_daemon(&proxy_server.base_url, &token).unwrap();

    let proxy = reqwest::Proxy::http(&proxy_url).unwrap();
    let client = reqwest::Client::builder().proxy(proxy).build().unwrap();
    let res = client
        .get(format!("{}/", target_server.base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn proxy_accepts_bearer_auth() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = setup_store(data_dir.path()).await;
    let state = build_state(
        data_dir.path(),
        stores,
        fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());
    let proxy_server = spawn_http_server(app).await;

    let target_app = Router::new().route("/", get(|| async { "ok" }));
    let target_server = spawn_http_server(target_app).await;

    let session_id = SessionId(Uuid::new_v4());
    let token = state.proxy_token_for_session(session_id).await;
    let target_url = Url::parse(&format!("{}/", target_server.base_url)).unwrap();
    let host = target_url.host_str().unwrap();
    let port = target_url.port_or_known_default().unwrap();

    let mut stream = TcpStream::connect(proxy_server.base_url.strip_prefix("http://").unwrap())
        .await
        .unwrap();
    let request = format!(
        "GET {url} HTTP/1.1\r\nHost: {host}:{port}\r\nProxy-Authorization: Bearer {token}\r\n\r\n",
        url = target_url,
    );
    stream.write_all(request.as_bytes()).await.unwrap();

    let headers = timeout(Duration::from_secs(2), read_headers(&mut stream))
        .await
        .unwrap();
    assert!(
        headers.contains(" 200 "),
        "expected 200 response, got: {headers}"
    );
}

#[tokio::test]
async fn proxy_connect_allows_full_profile() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = setup_store(data_dir.path()).await;
    let state = build_state(
        data_dir.path(),
        stores,
        fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());

    let target_addr = spawn_tcp_echo_server().await;

    let session_id = SessionId(Uuid::new_v4());
    let token = state.proxy_token_for_session(session_id).await;
    let creds = base64::engine::general_purpose::STANDARD.encode(format!("ctx:{token}"));

    let authority = format!("{}:{}", target_addr.ip(), target_addr.port());
    let target = format!("http://{authority}/");
    let req = axum::http::Request::builder()
        .method(Method::CONNECT)
        .uri(&target)
        .header(header::HOST, &authority)
        .header(header::PROXY_AUTHORIZATION, format!("Basic {creds}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn proxy_connect_blocks_none_profile() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = setup_store(data_dir.path()).await;
    let state = build_state(
        data_dir.path(),
        stores,
        fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());

    let settings = Settings {
        network: Some(NetworkSettings {
            profile: NetworkProfile::None,
            mcp_bypass: true,
        }),
        ..Settings::default()
    };
    ctx_http::settings::save_settings(data_dir.path(), &settings)
        .await
        .unwrap();

    let target_addr = spawn_tcp_echo_server().await;

    let session_id = SessionId(Uuid::new_v4());
    let token = state.proxy_token_for_session(session_id).await;
    let creds = base64::engine::general_purpose::STANDARD.encode(format!("ctx:{token}"));

    let authority = format!("{}:{}", target_addr.ip(), target_addr.port());
    let target = format!("http://{authority}/");
    let req = axum::http::Request::builder()
        .method(Method::CONNECT)
        .uri(&target)
        .header(header::HOST, &authority)
        .header(header::PROXY_AUTHORIZATION, format!("Basic {creds}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

async fn spawn_tcp_echo_server() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 4];
            if stream.read_exact(&mut buf).await.is_ok() && buf == *b"ping" {
                let _ = stream.write_all(b"pong").await;
            }
        }
    });
    addr
}

async fn read_headers(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}
