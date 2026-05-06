use super::*;

mod agent_server_config_errors;
mod streaming;
fn write_invalid_agent_server_config(data_root: &std::path::Path) {
    let path = data_root
        .join("providers")
        .join("agent-servers")
        .join("agent_servers.json");
    std::fs::create_dir_all(path.parent().expect("agent server config parent")).unwrap();
    std::fs::write(path, "{ not valid json").unwrap();
}

fn fake_default_session_payload() -> serde_json::Value {
    json!({
        "provider_id": "fake",
        "model_id": "fake-model",
        "execution_environment": "host",
    })
}

async fn create_fake_session_via_api(
    app: &axum::Router,
    git_repo_path: &str,
) -> ctx_core::models::Session {
    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": git_repo_path,
                "name": "ws"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let ws: ctx_core::models::Workspace = serde_json::from_slice(&body).unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/workspaces/{}/tasks", ws.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "title": "t1",
                "description": null,
                "default_session": fake_default_session_payload(),
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let task: ctx_core::models::Task = serde_json::from_slice(&body).unwrap();

    load_primary_session_via_api(app, &task).await
}

async fn build_fake_app_with_session(
    data_dir: &Path,
    git_repo_path: &str,
) -> (Arc<AppState>, axum::Router, ctx_core::models::Session) {
    let stores = StoreManager::open(data_dir).await.unwrap();
    let mut providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));

    let state = Arc::new(AppState::new(
        data_dir.to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let app = api::router(state.clone());
    let session = create_fake_session_via_api(&app, git_repo_path).await;
    (state, app, session)
}

async fn post_session_message_json(
    app: &axum::Router,
    session_id: ctx_core::ids::SessionId,
    payload: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sessions/{}/messages", session_id.0))
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let body = if body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    (status, body)
}

#[tokio::test]
async fn post_message_route_rejects_queueing_when_feature_flag_is_disabled() {
    let _serial = home_env_test_lock().lock().await;
    let _queueing = EnvVarGuard::set("CTX_QUEUED_MESSAGES_ENABLED", "0");
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());
    let data_dir = tempfile::tempdir().unwrap();
    let (state, app, session) =
        build_fake_app_with_session(data_dir.path(), &git_repo.path().to_string_lossy()).await;

    state.set_running(session.id, true).await;
    let (status, body) = post_session_message_json(
        &app,
        session.id,
        json!({ "content": "implicit delivery while running" }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        body.get("error").and_then(|value| value.as_str()),
        Some("A turn is already running. Stop it or wait for it to finish.")
    );

    state.set_running(session.id, true).await;
    let (status, body) = post_session_message_json(
        &app,
        session.id,
        json!({ "content": "explicit queued delivery", "delivery": "queued" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body.get("error").and_then(|value| value.as_str()),
        Some("Queued messages are disabled.")
    );

    let store = state.store_for_session(session.id).await.unwrap();
    let messages = store.list_messages_for_session(session.id).await.unwrap();
    assert!(
        messages.is_empty(),
        "rejected queue attempts must not persist messages"
    );
}

#[tokio::test]
async fn post_message_route_allows_queueing_when_feature_flag_is_enabled() {
    let _serial = home_env_test_lock().lock().await;
    let _queueing = EnvVarGuard::set("CTX_QUEUED_MESSAGES_ENABLED", "1");
    let git_repo = setup_git_repo().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());
    let data_dir = tempfile::tempdir().unwrap();
    let (state, app, session) =
        build_fake_app_with_session(data_dir.path(), &git_repo.path().to_string_lossy()).await;

    state.set_running(session.id, true).await;
    let (status, body) = post_session_message_json(
        &app,
        session.id,
        json!({ "content": "implicit queued delivery" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.get("delivery").and_then(|value| value.as_str()),
        Some("queued")
    );

    state.set_running(session.id, true).await;
    let (status, body) = post_session_message_json(
        &app,
        session.id,
        json!({ "content": "explicit queued delivery", "delivery": "queued" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.get("delivery").and_then(|value| value.as_str()),
        Some("queued")
    );
}

#[tokio::test]
async fn daemon_golden_path_with_fake_provider() {
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

    // create workspace
    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": git_repo.path().to_string_lossy(),
                "name": "ws"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let ws: ctx_core::models::Workspace = serde_json::from_slice(&body).unwrap();

    // create task (auto worktree when requested)
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/workspaces/{}/tasks", ws.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "title": "t1",
                "description": null,
                "default_session": fake_default_session_payload(),
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let task: ctx_core::models::Task = serde_json::from_slice(&body).unwrap();

    // load the default session created with the task
    let session = load_primary_session_via_api(&app, &task).await;

    // post message
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sessions/{}/messages", session.id.0))
        .header("content-type", "application/json")
        .body(Body::from(json!({"content":"hello"}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // wait for assistant message to be inserted
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let session_store = state.store_for_session(session.id).await.unwrap();
            let msgs = session_store
                .list_messages_for_session(session.id)
                .await
                .unwrap();
            if msgs
                .iter()
                .any(|m| matches!(m.role, ctx_core::models::MessageRole::Assistant))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("assistant message not produced"));
}

#[tokio::test]
async fn create_session_rejects_unknown_provider_id() {
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
    let app = api::router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": git_repo.path().to_string_lossy(),
                "name": "ws"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let ws: ctx_core::models::Workspace = serde_json::from_slice(&body).unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/workspaces/{}/tasks", ws.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "title": "t1",
                "default_session": fake_default_session_payload(),
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let task: ctx_core::models::Task = serde_json::from_slice(&body).unwrap();

    let primary_session_id = task
        .primary_session_id
        .expect("task should have a default session");
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/tasks/{}/sessions", task.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "provider_id": "not-a-provider",
                "model_id": "fake-model",
                "parent_session_id": primary_session_id.0.to_string(),
                "relationship": "sub_agent"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}
