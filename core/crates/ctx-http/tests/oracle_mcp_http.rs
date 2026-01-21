use axum::http::StatusCode;
use serde_json::json;
use std::sync::Arc;

use ctx_http::daemon::AppState;

mod common;

async fn setup() -> (tempfile::TempDir, Arc<AppState>, common::TestServer) {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = common::setup_store(data_dir.path()).await;
    let state = common::build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    let app = common::router(state.clone());
    let server = common::spawn_http_server(app).await;
    (data_dir, state, server)
}

#[tokio::test]
async fn oracle_rejects_invalid_session_id() {
    let (_data_dir, _state, server) = setup().await;
    let base = &server.base_url;
    let client = &server.client;

    let resp = client
        .post(format!("{base}/api/mcp/sessions/not-a-uuid/oracle"))
        .json(&json!({ "prompt": "what is 1+1" }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn oracle_rejects_nonexistent_session() {
    let (_data_dir, _state, server) = setup().await;
    let base = &server.base_url;
    let client = &server.client;

    let session_id = uuid::Uuid::new_v4();
    let resp = client
        .post(format!("{base}/api/mcp/sessions/{session_id}/oracle"))
        .json(&json!({ "prompt": "what is 1+1" }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "session not found");
}
