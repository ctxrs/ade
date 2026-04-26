use super::*;

#[tokio::test]
async fn session_state_exposes_artifact_metadata_and_session_scoped_downloads() {
    let _serial = home_env_test_lock().lock().await;
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());

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

    let wrong_session = create_subagent_session_via_api(&app, &task, session.id).await;

    let store = state.store_for_session(session.id).await.unwrap();
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .unwrap()
        .expect("session worktree");
    let artifact_path = std::path::PathBuf::from(worktree.root_path).join("artifact.txt");
    std::fs::write(&artifact_path, b"artifact-body\n").unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sessions/{}/artifacts", session.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "artifacts": [
                    {
                        "absolute_file_path": artifact_path.to_string_lossy(),
                        "name": "artifact.txt",
                        "mime_type": "text/plain"
                    }
                ]
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let artifacts: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let artifact = &artifacts[0];
    let artifact_id = artifact["id"].as_str().expect("artifact id");

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{}/state", session.id.0))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let session_state: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        session_state["artifacts"][0]["id"].as_str(),
        Some(artifact_id)
    );
    assert_eq!(
        session_state["artifacts"][0]["absolute_path"].as_str(),
        Some(artifact_path.to_string_lossy().as_ref())
    );

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            wrong_session.id.0, artifact_id
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .header(header::RANGE, "bytes=0-7")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        res.headers()
            .get(header::ACCEPT_RANGES)
            .and_then(|value| value.to_str().ok()),
        Some("bytes")
    );
    assert_eq!(
        res.headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("private, max-age=0, must-revalidate")
    );
    assert_eq!(
        res.headers()
            .get(header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok()),
        Some("bytes 0-7/14")
    );
    let etag = res
        .headers()
        .get(header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .expect("artifact range etag");
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"artifact");

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .header(header::RANGE, "Bytes= 0 - 7")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"artifact");

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .header(header::RANGE, "bytes=0-7")
        .header(header::IF_RANGE, etag.as_str())
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"artifact");

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .header(header::RANGE, "bytes=999-1000")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(
        res.headers()
            .get(header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok()),
        Some("bytes */14")
    );

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .header(header::RANGE, "bytes=999999999999999999999-")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::RANGE_NOT_SATISFIABLE);

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .header(header::RANGE, "items=0-1")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"artifact-body\n");

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .header(header::RANGE, "bytes=0-0,2-2")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"artifact-body\n");

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain")
    );
    assert_eq!(
        res.headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("private, max-age=0, must-revalidate")
    );
    assert_eq!(
        res.headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok()),
        Some(etag.as_str())
    );
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.as_ref(), b"artifact-body\n");

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .header(header::IF_NONE_MATCH, &etag)
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(
        res.headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok()),
        Some(etag.as_str())
    );
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert!(body.is_empty());

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/artifacts/{}",
            session.id.0, artifact_id
        ))
        .header(header::RANGE, "bytes=0-7")
        .header(header::IF_NONE_MATCH, &etag)
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_MODIFIED);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert!(body.is_empty());
}
