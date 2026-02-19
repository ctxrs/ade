#![allow(clippy::await_holding_lock)]

use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use axum::routing::get;
use axum::Json;
use bytes::Bytes;
use futures::stream;
use serde_json::{json, Value};

mod common;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(value) = self.previous.take() {
            std::env::set_var(self.key, value);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn current_platform_key() -> Option<&'static str> {
    ctx_http::updates::platform_key()
}

fn release_manifest_for(platform: &str, appimage_url_path: &str, sha256: &str) -> Value {
    json!({
      "channel": "stable",
      "latest_version": "9.9.9",
      "published_at": "2026-02-19T00:00:00Z",
      "platforms": {
        platform: {
          "appimage": {
            "url_path": appimage_url_path,
            "sha256": sha256
          },
          "daemon": {
            "url_path": "/download/stable/9.9.9/ctx-daemon",
            "sha256": "deadbeef"
          }
        }
      }
    })
}

async fn test_app_router(data_root: &std::path::Path) -> axum::Router {
    let stores = common::setup_store(data_root).await;
    let state = common::build_state(
        data_root.to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    common::router(state)
}

#[tokio::test]
async fn updates_check_rejects_malformed_manifest_metadata() {
    let _env_lock = lock_env();

    let malformed_manifest_server = common::spawn_http_server(axum::Router::new().route(
        "/releases/stable/latest.json",
        get(|| async { (StatusCode::OK, [("content-type", "application/json")], "{") }),
    ))
    .await;
    let _download_base =
        EnvGuard::set("CTX_DOWNLOAD_BASE_URL", &malformed_manifest_server.base_url);

    let data_dir = tempfile::tempdir().unwrap();
    let app = test_app_router(data_dir.path()).await;
    let (status, body): (StatusCode, Value) = common::json_request(
        &app,
        axum::http::Method::GET,
        "/api/updates/check?channel=stable",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::BAD_GATEWAY);
    let error = body
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase();
    assert!(
        error.contains("parsing release manifest json"),
        "unexpected error payload: {body}"
    );
}

#[tokio::test]
async fn download_update_rejects_checksum_mismatch_without_bricking_api() {
    let _env_lock = lock_env();

    let Some(platform) = current_platform_key() else {
        eprintln!("skipping: unsupported platform for updater checks");
        return;
    };

    let manifest = Arc::new(release_manifest_for(
        platform,
        "/download/stable/9.9.9/ctx.AppImage",
        "0000000000000000000000000000000000000000000000000000000000000000",
    ));

    let fake_release_server = common::spawn_http_server(
        axum::Router::new()
            .route(
                "/releases/stable/latest.json",
                get({
                    let manifest = Arc::clone(&manifest);
                    move || {
                        let manifest = Arc::clone(&manifest);
                        async move { Json((*manifest).clone()) }
                    }
                }),
            )
            .route(
                "/download/stable/9.9.9/ctx.AppImage",
                get(|| async {
                    (
                        StatusCode::OK,
                        [("content-type", "application/octet-stream")],
                        "definitely-not-the-right-hash",
                    )
                }),
            ),
    )
    .await;
    let _download_base = EnvGuard::set("CTX_DOWNLOAD_BASE_URL", &fake_release_server.base_url);

    let data_dir = tempfile::tempdir().unwrap();
    let app = test_app_router(data_dir.path()).await;

    let (status, body): (StatusCode, Value) = common::json_request(
        &app,
        axum::http::Method::POST,
        "/api/updates/appimage/download",
        Some(json!({ "channel": "stable" })),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_GATEWAY);
    let error = body
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase();
    assert!(
        error.contains("checksum mismatch"),
        "unexpected error payload: {body}"
    );

    let downloaded_path = data_dir.path().join("updates").join("ctx.AppImage.new");
    assert!(
        downloaded_path.exists(),
        "expected downloaded payload to remain at failure"
    );

    let (health_status, _health): (StatusCode, Value) =
        common::json_request(&app, axum::http::Method::GET, "/api/health", None).await;
    assert_eq!(health_status, StatusCode::OK);
}

#[tokio::test]
async fn download_update_rejects_missing_artifact_and_api_stays_healthy() {
    let _env_lock = lock_env();

    let Some(platform) = current_platform_key() else {
        eprintln!("skipping: unsupported platform for updater checks");
        return;
    };

    let manifest = Arc::new(release_manifest_for(
        platform,
        "/download/stable/9.9.9/missing.AppImage",
        "abc123",
    ));

    let fake_release_server = common::spawn_http_server(axum::Router::new().route(
        "/releases/stable/latest.json",
        get({
            let manifest = Arc::clone(&manifest);
            move || {
                let manifest = Arc::clone(&manifest);
                async move { Json((*manifest).clone()) }
            }
        }),
    ))
    .await;
    let _download_base = EnvGuard::set("CTX_DOWNLOAD_BASE_URL", &fake_release_server.base_url);

    let data_dir = tempfile::tempdir().unwrap();
    let app = test_app_router(data_dir.path()).await;

    let (status, body): (StatusCode, Value) = common::json_request(
        &app,
        axum::http::Method::POST,
        "/api/updates/appimage/download",
        Some(json!({ "channel": "stable" })),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_GATEWAY);
    let error = body
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase();
    assert!(
        error.contains("download http error"),
        "unexpected error payload: {body}"
    );

    let (health_status, _health): (StatusCode, Value) =
        common::json_request(&app, axum::http::Method::GET, "/api/health", None).await;
    assert_eq!(health_status, StatusCode::OK);
}

#[tokio::test]
async fn download_update_handles_interrupted_transfer_and_api_stays_healthy() {
    let _env_lock = lock_env();

    let Some(platform) = current_platform_key() else {
        eprintln!("skipping: unsupported platform for updater checks");
        return;
    };

    let manifest = Arc::new(release_manifest_for(
        platform,
        "/download/stable/9.9.9/interrupted.AppImage",
        "abc123",
    ));

    let fake_release_server = common::spawn_http_server(
        axum::Router::new()
            .route(
                "/releases/stable/latest.json",
                get({
                    let manifest = Arc::clone(&manifest);
                    move || {
                        let manifest = Arc::clone(&manifest);
                        async move { Json((*manifest).clone()) }
                    }
                }),
            )
            .route(
                "/download/stable/9.9.9/interrupted.AppImage",
                get(|| async move {
                    let body = axum::body::Body::from_stream(stream::iter(vec![
                        Ok::<Bytes, std::io::Error>(Bytes::from_static(b"partial-payload")),
                        Err(std::io::Error::new(
                            std::io::ErrorKind::ConnectionReset,
                            "simulated connection reset",
                        )),
                    ]));
                    axum::response::Response::builder()
                        .status(StatusCode::OK)
                        .header("content-type", "application/octet-stream")
                        .body(body)
                        .unwrap()
                }),
            ),
    )
    .await;
    let _download_base = EnvGuard::set("CTX_DOWNLOAD_BASE_URL", &fake_release_server.base_url);

    let data_dir = tempfile::tempdir().unwrap();
    let app = test_app_router(data_dir.path()).await;

    let (status, body): (StatusCode, Value) = common::json_request(
        &app,
        axum::http::Method::POST,
        "/api/updates/appimage/download",
        Some(json!({ "channel": "stable" })),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_GATEWAY);
    let error = body
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase();
    assert!(
        error.contains("reading download body") || error.contains("downloading:"),
        "unexpected error payload: {body}"
    );

    let downloaded_path = data_dir.path().join("updates").join("ctx.AppImage.new");
    assert!(
        !downloaded_path.exists(),
        "interrupted transfer should not leave a completed file at {}",
        downloaded_path.display()
    );

    let (health_status, _health): (StatusCode, Value) =
        common::json_request(&app, axum::http::Method::GET, "/api/health", None).await;
    assert_eq!(health_status, StatusCode::OK);
}
