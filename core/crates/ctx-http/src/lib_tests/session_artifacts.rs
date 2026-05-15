use super::*;

mod download_http;
mod missing_files;
mod root_paths;
mod spool_isolation;
#[cfg(unix)]
mod symlink_swap;

struct SessionArtifactFixture {
    _home_lock: tokio::sync::MutexGuard<'static, ()>,
    _home: EnvVarGuard,
    _home_dir: tempfile::TempDir,
    _data_dir: tempfile::TempDir,
    _git_repo: tempfile::TempDir,
    app: axum::Router,
    daemon: TestDaemon,
    task: ctx_core::models::Task,
    session: ctx_core::models::Session,
}

async fn build_session_artifact_fixture() -> SessionArtifactFixture {
    let home_lock = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home_dir = tempfile::tempdir().unwrap();
    let home = EnvVarGuard::set("HOME", &home_dir.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let daemon = test_daemon_with_fake_provider_for_test(data_dir.path(), None).await;
    let app = test_router(&daemon);

    let workspace = create_workspace_via_api(&app, &git_repo.path().to_string_lossy()).await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/workspaces/{}/tasks", workspace.id.0))
        .header("content-type", "application/json")
        .body(Body::from(json!({"title":"t1"}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let task: ctx_core::models::Task = serde_json::from_slice(&body).unwrap();

    let session = load_primary_session_via_api(&app, &task).await;

    SessionArtifactFixture {
        _home_lock: home_lock,
        _home: home,
        _home_dir: home_dir,
        _data_dir: data_dir,
        _git_repo: git_repo,
        app,
        daemon,
        task,
        session,
    }
}

async fn post_session_artifacts(
    app: &axum::Router,
    session_id: ctx_core::ids::SessionId,
    artifacts: serde_json::Value,
) -> axum::response::Response {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sessions/{}/artifacts", session_id.0))
        .header("content-type", "application/json")
        .body(Body::from(json!({ "artifacts": artifacts }).to_string()))
        .unwrap();
    app.clone().oneshot(req).await.unwrap()
}

async fn get_session_state(
    app: &axum::Router,
    session_id: ctx_core::ids::SessionId,
) -> serde_json::Value {
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{}/state", session_id.0))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn get_session_artifact(
    app: &axum::Router,
    session_id: ctx_core::ids::SessionId,
    artifact_id: ctx_core::ids::ArtifactId,
) -> axum::response::Response {
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session_id.0, artifact_id.0
        ))
        .body(Body::empty())
        .unwrap();
    app.clone().oneshot(req).await.unwrap()
}
