use super::*;

#[tokio::test]
async fn discard_edit_plan_returns_not_found_after_first_removal() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));
    let app = api::router(state.clone());

    let plan = crate::edit_plans::EditPlan {
        id: crate::edit_plans::EditPlanId::new(),
        session_id: ctx_core::ids::SessionId::new(),
        worktree_id: ctx_core::ids::WorktreeId::new(),
        title: "discard-me".to_string(),
        created_at: chrono::Utc::now(),
        worktree_root: data_dir.path().to_path_buf(),
        files: Vec::new(),
    };
    state
        .workspaces
        .edit_plans
        .lock()
        .await
        .insert(plan.id, plan.clone());

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/edit_plans/{}/discard", plan.id.0))
        .header(header::AUTHORIZATION, "Bearer daemon-secret")
        .body(Body::from("{}"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/edit_plans/{}/discard", plan.id.0))
        .header(header::AUTHORIZATION, "Bearer daemon-secret")
        .body(Body::from("{}"))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
