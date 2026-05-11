use super::*;

#[tokio::test]
async fn post_message_fails_turn_start_when_agent_server_config_is_invalid() {
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
    let session = create_fake_session_via_api(&app, &git_repo.path().to_string_lossy()).await;
    write_invalid_agent_server_config(data_dir.path());

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sessions/{}/messages", session.id.0))
        .header("content-type", "application/json")
        .body(Body::from(json!({"content":"hello"}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let session_store = state.store_for_session(session.id).await.unwrap();
            let turns = session_store
                .list_session_turns_page_by_seq(session.id, None, Some(10))
                .await
                .unwrap();
            if let Some(turn) = turns.last() {
                if turn.status == ctx_core::models::SessionTurnStatus::Failed {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("turn did not fail when agent server config was invalid"));
}

#[tokio::test]
async fn post_message_fails_turn_start_when_workspace_runtime_settings_are_invalid() {
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
    let session = create_fake_session_via_api(&app, &git_repo.path().to_string_lossy()).await;
    let session_store = state.store_for_session(session.id).await.unwrap();
    session_store
        .upsert_runtime_settings_document(1, "{ not valid json")
        .await
        .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sessions/{}/messages", session.id.0))
        .header("content-type", "application/json")
        .body(Body::from(json!({"content":"hello"}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let session_store = state.store_for_session(session.id).await.unwrap();
            let turns = session_store
                .list_session_turns_page_by_seq(session.id, None, Some(10))
                .await
                .unwrap();
            if let Some(turn) = turns.last() {
                if turn.status == ctx_core::models::SessionTurnStatus::Failed {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("turn did not fail when workspace runtime settings were invalid"));
}
