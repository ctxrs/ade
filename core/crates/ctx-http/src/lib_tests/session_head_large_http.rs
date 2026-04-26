use ctx_core::ids::{MessageId, RunId, SessionId, TaskId, TurnId};
use ctx_core::models::{
    Message, MessageDelivery, MessageRole, SessionEventType, SessionTurn, SessionTurnStatus,
    SessionTurnTool,
};
use ctx_providers::adapters::ProviderAdapter;
use ctx_store::Store;
use serde::de::DeserializeOwned;

use super::*;

#[tokio::test]
async fn large_session_head_http_responses_are_bounded() {
    let repo = setup_git_repo().await;
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
            "title": "Large Head",
            "create_default_session": false
        })),
    )
    .await;
    assert_eq!(task_status, StatusCode::OK);
    let (session_status, session): (StatusCode, ctx_core::models::Session) = json_request(
        &app,
        Method::POST,
        format!("/api/tasks/{}/sessions", task.id.0),
        Some(json!({
            "provider_id": "fake",
            "model_id": "fake-model"
        })),
    )
    .await;
    assert_eq!(session_status, StatusCode::OK);

    let store = state.store_for_session(session.id).await.unwrap();
    seed_large_session(&store, session.id, task.id, 240).await;

    let (heads_status, heads_body): (StatusCode, serde_json::Value) = json_request(
        &app,
        Method::GET,
        format!("/api/workspaces/{}/active_heads", workspace.id.0),
        None,
    )
    .await;
    assert_eq!(heads_status, StatusCode::OK, "{heads_body:#?}");
    let active_head = heads_body["heads"]
        .as_array()
        .expect("workspace active heads array")
        .iter()
        .find(|item| item["session"]["id"] == json!(session.id.0.to_string()))
        .expect("seeded session head present in workspace active heads");
    assert_eq!(
        active_head["turns"].as_array().map(Vec::len),
        Some(5),
        "workspace active heads should use the compact active-head window"
    );
    assert_eq!(active_head["has_more_turns"], json!(true));

    let (head_status, head_body): (StatusCode, serde_json::Value) = json_request(
        &app,
        Method::GET,
        format!(
            "/api/sessions/{}/head?limit=60&include_events=true",
            session.id.0
        ),
        None,
    )
    .await;
    assert_eq!(head_status, StatusCode::OK, "{head_body:#?}");
    assert_eq!(head_body["turns"].as_array().map(Vec::len), Some(60));
    assert_eq!(head_body["messages"].as_array().map(Vec::len), Some(60));
    assert_eq!(
        head_body["tool_summaries"].as_array().map(Vec::len),
        Some(60)
    );
    assert_eq!(head_body["has_more_turns"], json!(true));
    assert_eq!(head_body["messages"][0]["content"], json!("answer 180"));
    assert_eq!(head_body["messages"][59]["content"], json!("answer 239"));
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

async fn seed_large_session(store: &Store, session_id: SessionId, task_id: TaskId, turns: i64) {
    for index in 0..turns {
        let run_id = RunId::new();
        let turn_id = TurnId::new();
        let now = chrono::Utc::now();
        store
            .insert_session_turn(SessionTurn {
                turn_id,
                session_id,
                run_id: Some(run_id),
                user_message_id: None,
                status: SessionTurnStatus::Completed,
                start_seq: Some(index + 1),
                end_seq: Some(index + 1),
                started_at: now,
                updated_at: now,
                assistant_partial: None,
                thought_partial: None,
                metrics_json: None,
                tool_total: 1,
                tool_pending: 0,
                tool_running: 0,
                tool_completed: 1,
                tool_failed: 0,
            })
            .await
            .unwrap();
        let event = store
            .append_session_event(
                session_id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::Notice,
                json!({
                    "kind": "large_head_checkpoint",
                    "turn_index": index,
                }),
            )
            .await
            .unwrap();
        store
            .insert_message(Message {
                id: MessageId::new(),
                session_id,
                task_id,
                run_id: Some(run_id),
                turn_id: Some(turn_id),
                turn_sequence: Some(1),
                order_seq: None,
                role: MessageRole::Assistant,
                content: format!("answer {index}"),
                attachments: vec![],
                delivery: MessageDelivery::Immediate,
                delivered_at: None,
                created_at: now,
            })
            .await
            .unwrap();
        store
            .upsert_session_turn_tool(SessionTurnTool {
                session_id,
                tool_call_id: format!("tool-{index}"),
                turn_id,
                tool_kind: Some("execute".to_string()),
                provider_tool_name: Some("Bash".to_string()),
                title: Some("Bash".to_string()),
                subtitle: Some(format!("turn {index}")),
                status: Some("completed".to_string()),
                input_json: Some(json!({ "cmd": format!("echo {index}") })),
                output_text: Some(format!("output {index}")),
                order_seq: 1,
                first_event_seq: Some(event.seq),
                input_truncated: Some(false),
                input_original_bytes: None,
                output_truncated: Some(false),
                output_original_bytes: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
    }
}
