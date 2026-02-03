mod common;

use std::net::SocketAddr;

use axum::{routing::get, Router};
use base64::Engine;
use ctx_core::ids::WorkspaceId;
use ctx_http::egress_proxy::EgressProxy;
use ctx_http::ops_events::OpsEvents;
use ctx_http::settings::{ContainerNetworkMode, NetworkContext, NetworkProfile};
use http::StatusCode;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use url::Url;

async fn proxy_get(proxy_addr: SocketAddr, token: &str, url: &Url) -> StatusCode {
    let auth = base64::engine::general_purpose::STANDARD.encode(format!("ctx-proxy:{token}"));
    let host = url.host_str().expect("url host should be present");
    let port = url.port_or_known_default().unwrap_or(80);
    let authority = if port == 80 {
        host.to_string()
    } else {
        format!("{host}:{port}")
    };
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nProxy-Authorization: Basic {}\r\nConnection: close\r\n\r\n",
        url.as_str(),
        authority,
        auth
    );

    let mut stream = TcpStream::connect(proxy_addr).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let response = String::from_utf8_lossy(&response);
    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .expect("proxy response should include status code");
    StatusCode::from_u16(status).unwrap()
}

#[tokio::test]
async fn egress_proxy_respects_allowlist_profiles() {
    let data_root = tempfile::tempdir().unwrap();
    let ops_events = OpsEvents::new(data_root.path().to_path_buf());
    let proxy = EgressProxy::spawn(ops_events).unwrap();

    let server = common::spawn_http_server(Router::new().route("/", get(|| async { "ok" }))).await;
    let url = Url::parse(&format!("{}/", server.base_url)).unwrap();
    let host = url.host_str().unwrap().to_string();

    let allow_profile = NetworkProfile {
        mode: ContainerNetworkMode::Allowlist,
        allowlist: vec![host.clone()],
    };
    let allow_token = proxy
        .issue_context_token(
            WorkspaceId::new(),
            NetworkContext::AgentDefault,
            allow_profile,
        )
        .await;
    let allowed_status = proxy_get(proxy.addr(), &allow_token, &url).await;
    assert_eq!(allowed_status, StatusCode::OK);

    let deny_profile = NetworkProfile {
        mode: ContainerNetworkMode::LlmOnly,
        allowlist: Vec::new(),
    };
    let deny_token = proxy
        .issue_context_token(
            WorkspaceId::new(),
            NetworkContext::AgentDefault,
            deny_profile,
        )
        .await;
    let denied_status = proxy_get(proxy.addr(), &deny_token, &url).await;
    assert_eq!(denied_status, StatusCode::FORBIDDEN);
}
