#![allow(clippy::await_holding_lock)]

use axum::http::StatusCode;
use axum::routing::get;
use serde_json::Value;

mod common;

use common::updates_failure_safety::{lock_env, test_app_router, EnvGuard};

#[tokio::test]
async fn updates_check_rejects_unsigned_manifest_metadata() {
    let _env_lock = lock_env();

    let unsigned_manifest_server = common::spawn_http_server(axum::Router::new().route(
        "/releases/stable/latest.json",
        get(|| async {
            (
                StatusCode::OK,
                [("content-type", "application/json")],
                r#"{"channel":"stable","latest_version":"9.9.9","published_at":"2026-02-19T00:00:00Z","platforms":{"linux-x64":{"appimage":{"url_path":"/download/stable/9.9.9/ctx.AppImage","sha256":"1111111111111111111111111111111111111111111111111111111111111111"},"daemon":{"url_path":"/download/stable/9.9.9/ctx-daemon","sha256":"2222222222222222222222222222222222222222222222222222222222222222"}}}}"#,
            )
        }),
    ))
    .await;
    let _download_base = EnvGuard::set("CTX_DOWNLOAD_BASE_URL", &unsigned_manifest_server.base_url);

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
        error.contains("signature"),
        "unexpected error payload: {body}"
    );
}
