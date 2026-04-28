use super::*;

mod browser_capability_tokens;
mod browser_http_bearers;
mod browser_stream_tokens;
mod daemon_http;
mod mcp_session_scope;
mod mobile_tokens;
mod terminal_stream_tokens;

async fn serve_test_app(app: axum::Router) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, server)
}

async fn websocket_upgrade_status(
    client: &reqwest::Client,
    addr: std::net::SocketAddr,
    path: &str,
) -> StatusCode {
    client
        .get(format!("http://{addr}{path}"))
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await
        .unwrap()
        .status()
}
