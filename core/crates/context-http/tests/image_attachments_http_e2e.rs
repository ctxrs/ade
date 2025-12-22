use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use base64::Engine;
use context_http::api;
use context_http::daemon::AppState;
use context_providers::fake::FakeProviderAdapter;
use context_store::Store;
use serde_json::json;
use tower::ServiceExt;

async fn run_git(repo: &Path, args: &[&str]) {
    let out = tokio::process::Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .await
        .expect("git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

async fn create_test_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    run_git(root, &["init"]).await;
    run_git(root, &["config", "user.email", "test@example.com"]).await;
    run_git(root, &["config", "user.name", "Test"]).await;
    std::fs::write(root.join("README.md"), "hello\n").unwrap();
    run_git(root, &["add", "."]).await;
    run_git(root, &["commit", "-m", "init"]).await;
    dir
}

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
    let db_dir = data_dir.path().join("db");
    tokio::fs::create_dir_all(&db_dir).await.unwrap();
    let store = Store::open(db_dir.join("db.sqlite")).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn context_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        store,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());

    // 1) Upload blob and fetch it back.
    let boundary = "context-test-boundary";
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

    // 2) Create workspace/task/track/session and post message with legacy base64 attachment;
    //    server should normalize it to image_ref before persisting.
    let repo = create_test_repo().await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": repo.path().to_string_lossy(),
                "name": "ws"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let ws: context_core::models::Workspace = serde_json::from_slice(&body).unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/workspaces/{}/tasks", ws.id.0))
        .header("content-type", "application/json")
        .body(Body::from(json!({"title":"t1"}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/workspaces/{}/tasks", ws.id.0))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let tasks: Vec<context_core::models::Task> = serde_json::from_slice(&body).unwrap();
    let task_id = tasks[0].id.0;

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/tasks/{task_id}/tracks"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"label":"t"}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let track: context_core::models::Track = serde_json::from_slice(&body).unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/tracks/{}/sessions", track.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"provider_id":"fake","model_id":"fake-model"}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let session: context_core::models::Session = serde_json::from_slice(&body).unwrap();

    let data_base64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sessions/{}/messages", session.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "content":"hi",
                "delivery":"queued",
                "attachments":[{"kind":"image","mime_type":"image/png","data_base64":data_base64,"name":"x.png"}]
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let msg: context_core::models::Message = serde_json::from_slice(&body).unwrap();

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
