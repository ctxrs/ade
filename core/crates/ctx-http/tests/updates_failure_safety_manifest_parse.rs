#![allow(clippy::await_holding_lock)]

use axum::http::StatusCode;
use axum::routing::get;
use serde_json::Value;

mod common;

use common::updates_failure_safety::{
    lock_env, sign_release_manifest_body, test_app_router, EnvGuard,
};

#[tokio::test]
async fn updates_check_rejects_malformed_manifest_metadata() {
    let _env_lock = lock_env();
    let (signature_b64, pubkey_b64) = sign_release_manifest_body("{");

    let malformed_manifest_server = common::spawn_http_server(
        axum::Router::new()
            .route(
                "/releases/stable/latest.json",
                get(|| async { (StatusCode::OK, [("content-type", "application/json")], "{") }),
            )
            .route(
                "/releases/stable/latest.json.sig",
                get(move || {
                    let signature_b64 = signature_b64.clone();
                    async move {
                        (
                            StatusCode::OK,
                            [("content-type", "text/plain")],
                            signature_b64,
                        )
                    }
                }),
            ),
    )
    .await;
    let _download_base =
        EnvGuard::set("CTX_DOWNLOAD_BASE_URL", &malformed_manifest_server.base_url);
    let _manifest_pubkey = EnvGuard::set("CTX_RELEASE_MANIFEST_PUBKEY", &pubkey_b64);

    let data_dir = tempfile::tempdir().unwrap();
    let app_harness = test_app_router(data_dir.path()).await;
    let app = app_harness.app();
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
