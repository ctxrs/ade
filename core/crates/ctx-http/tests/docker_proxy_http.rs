mod common;

use std::path::Path;

use axum::body::Body;
use ctx_core::ids::SessionId;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::Response;
use hyper_util::rt::TokioIo;
use tokio::net::UnixListener;
use uuid::Uuid;

use common::{build_state, fake_providers, setup_store, spawn_http_server};

struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(prev) = &self.prev {
            std::env::set_var(self.key, prev);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn set_env(key: &'static str, value: &str) -> EnvGuard {
    let prev = std::env::var(key).ok();
    std::env::set_var(key, value);
    EnvGuard { key, prev }
}

async fn spawn_unix_http_server(socket_path: &Path) {
    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }
    let listener = UnixListener::bind(socket_path).unwrap();
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let io = TokioIo::new(stream);
            let service = service_fn(|_req| async move {
                Ok::<_, std::convert::Infallible>(
                    Response::builder()
                        .status(200)
                        .body(Body::from("OK"))
                        .unwrap(),
                )
            });
            let _ = http1::Builder::new().serve_connection(io, service).await;
        }
    });
}

#[tokio::test]
async fn docker_proxy_requires_auth() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = setup_store(data_dir.path()).await;
    let state = build_state(
        data_dir.path(),
        stores,
        fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state);
    let server = spawn_http_server(app).await;

    let res = server
        .client
        .get(format!("{}/_ping", server.base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn docker_proxy_forwards_with_auth() {
    let temp_dir = tempfile::tempdir().unwrap();
    let socket_path = temp_dir.path().join("docker.sock");
    spawn_unix_http_server(&socket_path).await;
    let _guard = set_env(
        "CTX_DOCKER_PROXY_UPSTREAM",
        socket_path.to_string_lossy().as_ref(),
    );

    let data_dir = tempfile::tempdir().unwrap();
    let stores = setup_store(data_dir.path()).await;
    let state = build_state(
        data_dir.path(),
        stores,
        fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());
    let server = spawn_http_server(app).await;

    let session_id = SessionId(Uuid::new_v4());
    let token = state.docker_proxy_token_for_session(session_id).await;
    let res = server
        .client
        .get(format!("{}/_ping", server.base_url))
        .basic_auth("ctx", Some(token))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let body = res.text().await.unwrap();
    assert_eq!(body, "OK");
}

#[tokio::test]
async fn docker_host_injection_requires_opt_in() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = setup_store(data_dir.path()).await;
    let state = build_state(
        data_dir.path(),
        stores,
        fake_providers(),
        "http://127.0.0.1:4399",
    );
    let session_id = SessionId(Uuid::new_v4());

    let host = ctx_http::docker_proxy::docker_host_for_session(&state, session_id).await;
    assert!(host.is_none());

    let _guard = set_env("CTX_DOCKER_PROXY_ENABLED", "1");
    let host = ctx_http::docker_proxy::docker_host_for_session(&state, session_id)
        .await
        .unwrap();
    assert!(host.starts_with("tcp://ctx:"));
    assert!(host.contains("@127.0.0.1:4399"));
}
