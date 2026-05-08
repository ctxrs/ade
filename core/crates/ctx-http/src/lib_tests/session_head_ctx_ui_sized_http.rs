use ctx_providers::adapters::ProviderAdapter;
use serde::de::DeserializeOwned;
use std::time::{Duration, Instant};

use super::*;
use seed::{latest_turn_id, seed_ctx_ui_sized_session, tail_turn_ids, CtxUiSizedSeed};

mod seed;

#[tokio::test]
async fn ctx_ui_sized_active_session_head_recovery_is_bounded() {
    const TURN_COUNT: i64 = 68;
    const HEAD_LIMIT: i64 = 60;
    const TOOL_COUNT: i64 = 16_320;
    const MESSAGE_COUNT: i64 = 5_880;
    const EVENT_COUNT: i64 = 65_600;
    const TOOL_OUTPUT_BYTES: usize = 4 * 1024;
    const TOOL_SUMMARY_LIMIT: usize = 96;
    const HEAD_BYTE_LIMIT: usize = 256_000;
    let head_recovery_budget = Duration::from_secs(2);
    let step_timeout = Duration::from_secs(180);
    let _serial = home_env_test_lock().lock().await;

    let repo = setup_git_repo().await;
    let _projection_flush_ms = EnvVarGuard::set("CTX_ACTIVE_HEAD_PROJECTION_FLUSH_MS", "600000");
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    let app = api::router(state.clone());

    let workspace = create_workspace_via_api(&app, &repo.path().to_string_lossy()).await;
    let (task_status, task): (StatusCode, ctx_core::models::Task) = json_request(
        &app,
        Method::POST,
        format!("/api/workspaces/{}/tasks", workspace.id.0),
        Some(json!({
            "title": "ctx-ui sized active recovery",
            "default_session": {
                "provider_id": "fake",
                "model_id": "fake-model"
            }
        })),
    )
    .await;
    assert_eq!(task_status, StatusCode::OK);
    let session = load_primary_session_via_api(&app, &task).await;

    let store = state.store_for_session(session.id).await.unwrap();
    seed_ctx_ui_sized_session(
        &store,
        session.id,
        task.id,
        CtxUiSizedSeed {
            turn_count: TURN_COUNT,
            message_count: MESSAGE_COUNT,
            tool_count: TOOL_COUNT,
            event_count: EVENT_COUNT,
            tool_output_bytes: TOOL_OUTPUT_BYTES,
        },
    )
    .await;
    let event_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM session_events WHERE session_id = ?")
            .bind(session.id.0.to_string())
            .fetch_one(store.pool())
            .await
            .unwrap();
    let tool_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM session_turn_tools WHERE session_id = ?")
            .bind(session.id.0.to_string())
            .fetch_one(store.pool())
            .await
            .unwrap();
    let message_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE session_id = ?")
            .bind(session.id.0.to_string())
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(event_count, EVENT_COUNT);
    assert_eq!(tool_count, TOOL_COUNT);
    assert_eq!(message_count, MESSAGE_COUNT);

    tokio::time::timeout(
        step_timeout,
        store.refresh_active_session_head_projection(session.id),
    )
    .await
    .unwrap_or_else(|_| panic!("timed out refreshing active projection for ctx-ui fixture"))
    .unwrap();
    tokio::time::timeout(
        step_timeout,
        state.ensure_workspace_active_snapshot_hydrated(workspace.id),
    )
    .await
    .unwrap_or_else(|_| panic!("timed out hydrating workspace active snapshot"))
    .unwrap();

    let started = Instant::now();
    let (head_status, head_body): (StatusCode, serde_json::Value) = tokio::time::timeout(
        step_timeout,
        json_request(
            &app,
            Method::GET,
            format!(
                "/api/sessions/{}/head?limit={HEAD_LIMIT}&include_events=true",
                session.id.0,
            ),
            None,
        ),
    )
    .await
    .unwrap_or_else(|_| panic!("timed out requesting ctx-ui sized active session head"));
    let elapsed = started.elapsed();

    assert_eq!(head_status, StatusCode::OK, "{head_body:#?}");
    eprintln!(
        "ctx-ui-sized-head elapsed_ms={} events={} tools={} messages={} response_bytes={}",
        elapsed.as_millis(),
        event_count,
        tool_count,
        message_count,
        serde_json::to_vec(&head_body).unwrap().len(),
    );
    assert!(
        elapsed <= head_recovery_budget,
        "ctx-ui-sized /head recovery took {}ms, over {}ms budget",
        elapsed.as_millis(),
        head_recovery_budget.as_millis()
    );
    assert!(head_body["has_more_turns"].as_bool().unwrap_or(false));
    assert!(
        head_body["turns"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default()
            <= HEAD_LIMIT as usize,
        "head turns must stay bounded"
    );
    assert!(
        head_body["tool_summaries"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default()
            <= TOOL_SUMMARY_LIMIT,
        "head tool summaries must stay bounded"
    );
    assert_eq!(
        head_body["head_window"]["truncated"],
        json!(true),
        "ctx-ui sized head should report that long-tail state was intentionally truncated"
    );
    assert!(
        head_body["head_window"]["bytes"]
            .as_i64()
            .unwrap_or(i64::MAX)
            <= HEAD_BYTE_LIMIT as i64,
        "head window bytes must stay within the bounded recovery policy"
    );
    let latest_turn_id = latest_turn_id(&store, session.id).await;
    let latest_visible_tool_count = head_body["tool_summaries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|tool| tool["turn_id"].as_str() == Some(latest_turn_id.0.to_string().as_str()))
        .count();
    assert!(
        latest_visible_tool_count > 0,
        "head must rebuild missing tool summaries for the latest visible running turn"
    );

    let tail_turn_ids = tail_turn_ids(&store, session.id, HEAD_LIMIT).await;
    let bounded_tools = store
        .list_recent_turn_tool_summaries_for_turns(session.id, &tail_turn_ids, TOOL_SUMMARY_LIMIT)
        .await
        .unwrap();
    assert_eq!(
        bounded_tools.len(),
        TOOL_SUMMARY_LIMIT + 1,
        "tool-summary recovery must fetch exactly one sentinel row past the visible limit"
    );
    let oldest_loaded_order_seq = bounded_tools
        .iter()
        .map(|tool| tool.order_seq)
        .min()
        .unwrap_or(i64::MIN);
    assert!(
        oldest_loaded_order_seq >= TOOL_COUNT - (TOOL_SUMMARY_LIMIT as i64 + 1),
        "tool-summary recovery must seek into the latest hot rows, not load the long tail"
    );
}

async fn json_request<T: DeserializeOwned>(
    app: &axum::Router,
    method: Method,
    uri: impl Into<String>,
    body: Option<serde_json::Value>,
) -> (StatusCode, T) {
    let req = Request::builder()
        .method(method)
        .uri(uri.into())
        .header("content-type", "application/json")
        .body(Body::from(
            body.unwrap_or(serde_json::Value::Null).to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let parsed = serde_json::from_slice(&body).unwrap_or_else(|err| {
        panic!(
            "failed to parse JSON response (status {}): {}\nbody: {}",
            status,
            err,
            String::from_utf8_lossy(&body)
        )
    });
    (status, parsed)
}
