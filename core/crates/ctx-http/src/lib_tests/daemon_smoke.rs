use super::*;

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
            json!({"title":"t1","description":null}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let task: ctx_core::models::Task = serde_json::from_slice(&body).unwrap();

    // create session
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/tasks/{}/sessions", task.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"provider_id":"fake","model_id":"fake-model"}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let session: ctx_core::models::Session = serde_json::from_slice(&body).unwrap();

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
async fn daemon_http_and_ws_streaming() {
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
    {
        let mut statuses = HashMap::new();
        statuses.insert(
            "fake".into(),
            ProviderStatus {
                provider_id: "fake".into(),
                installed: true,
                detected_path: None,
                version: Some("0.1.0".into()),
                capabilities: None,
                health: ctx_providers::adapters::ProviderHealth::Ok,
                diagnostics: vec![],
                details: HashMap::new(),
                usability: ctx_providers::adapters::ProviderUsability::default(),
            },
        );
        *state.providers.statuses.lock().await = statuses;
    }

    let app = api::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    // providers endpoint
    let providers_res: Vec<ProviderStatus> = client
        .get(format!("{base}/api/providers"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(providers_res.len(), 1);

    // create workspace
    let ws: ctx_core::models::Workspace = client
        .post(format!("{base}/api/workspaces"))
        .json(&json!({
            "root_path": git_repo.path().to_string_lossy(),
            "name": "ws"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // create task
    let task: ctx_core::models::Task = client
        .post(format!("{base}/api/workspaces/{}/tasks", ws.id.0))
        .json(&json!({"title":"t1"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // create session
    let session: ctx_core::models::Session = client
        .post(format!("{base}/api/tasks/{}/sessions", task.id.0))
        .json(&json!({"provider_id":"fake","model_id":"fake-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // open workspace stream before sending message
    let ws_url = format!("ws://{}/api/workspaces/{}/stream", addr, ws.id.0);
    let (mut ws_stream, _) = tokio_tungstenite::connect_async(ws_url).await.unwrap();
    let subscribe = serde_json::json!({
        "type": "subscribe",
        "sessions": [{
            "session_id": session.id.0,
            "replay": {
                "mode": "resume",
                "after_seq": 0,
            },
        }],
    })
    .to_string();
    ws_stream
        .send(tokio_tungstenite::tungstenite::Message::Text(
            subscribe.into(),
        ))
        .await
        .unwrap();

    // post message
    let _msg: ctx_core::models::Message = client
        .post(format!("{base}/api/sessions/{}/messages", session.id.0))
        .json(&json!({"content":"hello"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // consume workspace stream until we see a Done event for the session
    let seen_done = tokio::time::timeout(Duration::from_secs(20), async {
        let mut seen_done = false;
        while let Some(Ok(frame)) = ws_stream.next().await {
            if let tokio_tungstenite::tungstenite::Message::Text(txt) = frame {
                let message: ctx_core::models::WorkspaceActiveSnapshotStreamMessage =
                    serde_json::from_str(&txt).unwrap_or_else(|err| {
                        panic!("failed to decode workspace stream message: {err}; raw={txt}")
                    });
                match message {
                    ctx_core::models::WorkspaceActiveSnapshotStreamMessage::Event {
                        event, ..
                    } => match event.as_ref() {
                        ctx_core::models::WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                            delta,
                            ..
                        } => {
                            let is_done = delta.session_id == session.id
                                && delta
                                    .event
                                    .as_ref()
                                    .map(|event| {
                                        matches!(
                                            event.event_type,
                                            ctx_core::models::SessionEventType::Done
                                        )
                                    })
                                    .unwrap_or(false);
                            if is_done {
                                seen_done = true;
                                break;
                            }
                        }
                        ctx_core::models::WorkspaceActiveSnapshotEvent::SessionHeadSeed {
                            head,
                            ..
                        } => {
                            let is_done = head.session.id == session.id
                                && head.events.iter().any(|event| {
                                    matches!(
                                        event.event_type,
                                        ctx_core::models::SessionEventType::Done
                                    )
                                });
                            if is_done {
                                seen_done = true;
                                break;
                            }
                        }
                        _ => {}
                    },
                    ctx_core::models::WorkspaceActiveSnapshotStreamMessage::HeadsBatch {
                        deltas,
                        ..
                    } => {
                        for delta in deltas {
                            if delta.session_id != session.id {
                                continue;
                            }
                            let is_done = delta
                                .event
                                .as_ref()
                                .map(|event| {
                                    matches!(
                                        event.event_type,
                                        ctx_core::models::SessionEventType::Done
                                    )
                                })
                                .unwrap_or(false);
                            if is_done {
                                seen_done = true;
                                break;
                            }
                        }
                        if seen_done {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        seen_done
    })
    .await
    .expect("timed out waiting for Done event");
    assert!(seen_done);

    let store = state.store_for_session(session.id).await.unwrap();
    let events = store.list_session_events(session.id).await.unwrap();
    assert!(events.iter().any(|e| matches!(
        e.event_type,
        ctx_core::models::SessionEventType::UserMessage
    )));

    // mark task read/unread endpoints
    let task_after_read: ctx_core::models::Task = client
        .post(format!("{base}/api/tasks/{}/mark_read", task.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(task_after_read.assistant_seen_at.is_some());

    let task_after_unread: ctx_core::models::Task = client
        .post(format!("{base}/api/tasks/{}/mark_unread", task.id.0))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(task_after_unread.assistant_seen_at.is_none());

    server.abort();
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
        .body(Body::from(json!({"title":"t1"}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let task: ctx_core::models::Task = serde_json::from_slice(&body).unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/tasks/{}/sessions", task.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"provider_id":"not-a-provider","model_id":"fake-model"}).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}
