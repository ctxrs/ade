use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use tower::ServiceExt;

use ctx_core::models::SessionEventType;
use ctx_http::daemon::AppState;
use ctx_providers::adapters::ProviderAdapter;
use ctx_providers::crp::Tier1CrpAdapter;
use ctx_store::StoreManager;

mod common;

async fn post_message(app: &axum::Router, session_id: uuid::Uuid, content: &str) {
    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/sessions/{session_id}/messages"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "content": content }).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

async fn wait_for_terminal(state: &Arc<AppState>, session_id: ctx_core::ids::SessionId) {
    let store = state.store_for_session(session_id).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(240);
    loop {
        let events = store.list_session_events(session_id).await.unwrap();
        if events
            .iter()
            .any(|e| matches!(e.event_type, SessionEventType::Done))
        {
            return;
        }
        if events.iter().any(|e| {
            matches!(
                e.event_type,
                SessionEventType::Error | SessionEventType::AuthRequired
            )
        }) {
            panic!("live canary saw terminal error/auth-required events: {events:#?}");
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for Done event: {events:#?}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test]
#[ignore]
async fn live_provider_canary_turn_invariants() {
    let provider_id = std::env::var("CTX_LIVE_PROVIDER_ID").ok();
    let model_id = std::env::var("CTX_LIVE_MODEL_ID").ok();
    if provider_id.is_none() || model_id.is_none() {
        eprintln!("skipping: set CTX_LIVE_PROVIDER_ID and CTX_LIVE_MODEL_ID to run live canaries");
        return;
    }
    let provider_id = provider_id.unwrap();
    let model_id = model_id.unwrap();

    let adapter: Arc<dyn ProviderAdapter> = match provider_id.as_str() {
        "codex" | "codex-crp" => Arc::new(Tier1CrpAdapter::codex()),
        "claude" | "claude-crp" => Arc::new(Tier1CrpAdapter::claude()),
        _ => {
            eprintln!(
                "skipping: CTX_LIVE_PROVIDER_ID={provider_id} not supported by this canary yet"
            );
            return;
        }
    };

    let repo = common::init_git_repo(&[("note.txt", "hello\n")]).await;
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();

    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert(provider_id.clone(), adapter);

    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app = ctx_http::api::router(state.clone());
    let ws = common::create_workspace(&app, repo.path(), "ws").await;
    let task = common::create_task(&app, ws.id.0, "t1").await;
    let session = common::create_session(&app, task.id.0, &provider_id, &model_id).await;

    post_message(&app, session.id.0, "Say hi in one short sentence.").await;
    wait_for_terminal(&state, session.id).await;

    let store = state.store_for_session(session.id).await.unwrap();
    let events = store.list_session_events(session.id).await.unwrap();

    let assistant_finals = events
        .iter()
        .filter(|e| matches!(e.event_type, SessionEventType::AssistantComplete))
        .count();
    assert!(
        assistant_finals >= 1,
        "expected at least 1 AssistantComplete; saw {assistant_finals}: {events:#?}"
    );
}
