use std::path::PathBuf;

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use base64::Engine;
use serde_json::json;
use tower::ServiceExt;

mod common;

fn multipart_body(
    boundary: &str,
    name: &str,
    filename: &str,
    content_type: &str,
    bytes: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    out.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    out.extend_from_slice(format!("Content-Type: {content_type}\r\n\r\n").as_bytes());
    out.extend_from_slice(bytes);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    out
}

#[tokio::test]
async fn image_attachments_use_blobs_and_never_persist_base64() {
    // Tiny 1x1 PNG.
    const PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMB/6Vn3b0AAAAASUVORK5CYII=";
    let png_bytes = base64::engine::general_purpose::STANDARD
        .decode(PNG_BASE64.as_bytes())
        .unwrap();

    let data_dir = tempfile::tempdir().unwrap();
    let store = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        store,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());

    // 1) Upload blob and fetch it back.
    let boundary = "ctx-test-boundary";
    let body = multipart_body(boundary, "file", "x.png", "image/png", &png_bytes);
    let req = Request::builder()
        .method("POST")
        .uri("/api/blobs")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let uploaded: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let uploaded_id = uploaded.get("blob_id").and_then(|v| v.as_str()).unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/blobs/{uploaded_id}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        "image/png"
    );
    let fetched = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(fetched.as_ref(), png_bytes.as_slice());

    // 2) Create workspace/task/session and post message with legacy base64 attachment;
    //    server should normalize it to image_ref before persisting.
    let repo = common::init_git_repo(&[("README.md", "hello\n")]).await;

    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let task = common::create_task(&app, ws.id.0, "t1").await;
    let task_id = task.id.0;

    let session = common::create_session(&app, task_id, "fake", "fake-model").await;

    let data_base64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
    let (status, msg): (StatusCode, ctx_core::models::Message) = common::json_request(
        &app,
        Method::POST,
        format!("/api/sessions/{}/messages", session.id.0),
        Some(json!({
            "content":"hi",
            "delivery":"queued",
            "attachments":[{"kind":"image","mime_type":"image/png","data_base64":data_base64,"name":"x.png"}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(msg.attachments.len(), 1);
    let att_json = serde_json::to_value(&msg.attachments[0]).unwrap();
    assert_eq!(
        att_json.get("kind").and_then(|v| v.as_str()),
        Some("image_ref")
    );
    assert!(
        att_json.get("data_base64").is_none(),
        "expected no base64 persisted in message attachment: {att_json:?}"
    );

    let ref_blob_id = att_json.get("blob_id").and_then(|v| v.as_str()).unwrap();
    let blob_path: PathBuf = data_dir.path().join("blobs").join(ref_blob_id);
    assert!(blob_path.exists(), "expected blob file at {blob_path:?}");
}
