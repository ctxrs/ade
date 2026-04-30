use ctx_core::ids::{MessageId, RunId, SessionId, TaskId, TurnId};
use ctx_providers::adapters::ProviderAdapter;
use ctx_store::Store;
use serde::de::DeserializeOwned;
use sqlx::{QueryBuilder, Sqlite};

use super::*;

#[tokio::test]
async fn large_session_head_http_responses_are_bounded() {
    const SEEDED_TURNS: i64 = 65;
    const HEAD_LIMIT: i64 = 60;
    let step_timeout = std::time::Duration::from_secs(120);
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
            "title": "Large Head",
            "default_session": {
                "provider_id": "fake",
                "model_id": "fake-model"
            }
        })),
    )
    .await;
    assert_eq!(task_status, StatusCode::OK);
    let (session_status, sessions): (StatusCode, Vec<ctx_core::models::Session>) = json_request(
        &app,
        Method::GET,
        format!("/api/tasks/{}/sessions", task.id.0),
        None,
    )
    .await;
    assert_eq!(session_status, StatusCode::OK);
    let session = sessions
        .into_iter()
        .find(|session| Some(session.id) == task.primary_session_id)
        .expect("created task should list its default session");

    let store = state.store_for_session(session.id).await.unwrap();
    seed_large_session(&store, session.id, task.id, SEEDED_TURNS).await;
    // Keep the bulk seed deterministic without waiting for unrelated queued
    // projection work from background refresh scheduling.
    tokio::time::timeout(
        step_timeout,
        store.refresh_active_session_head_projection(session.id),
    )
    .await
    .unwrap_or_else(|_| panic!("timed out refreshing active session head projection"))
    .unwrap();
    tokio::time::timeout(
        step_timeout,
        state.ensure_workspace_active_snapshot_hydrated(workspace.id),
    )
    .await
    .unwrap_or_else(|_| panic!("timed out hydrating workspace active snapshot"))
    .unwrap();

    let (heads_status, heads_body): (StatusCode, serde_json::Value) = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        json_request(
            &app,
            Method::GET,
            format!("/api/workspaces/{}/active_heads", workspace.id.0),
            None,
        ),
    )
    .await
    .unwrap_or_else(|_| panic!("timed out requesting workspace active heads"));
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

    let (head_status, head_body): (StatusCode, serde_json::Value) = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        json_request(
            &app,
            Method::GET,
            format!(
                "/api/sessions/{}/head?limit=60&include_events=true",
                session.id.0
            ),
            None,
        ),
    )
    .await
    .unwrap_or_else(|_| panic!("timed out requesting session head"));
    assert_eq!(head_status, StatusCode::OK, "{head_body:#?}");
    assert_eq!(head_body["turns"].as_array().map(Vec::len), Some(60));
    assert_eq!(head_body["messages"].as_array().map(Vec::len), Some(60));
    assert_eq!(
        head_body["tool_summaries"].as_array().map(Vec::len),
        Some(60)
    );
    assert_eq!(head_body["has_more_turns"], json!(true));
    assert_eq!(
        head_body["messages"][0]["content"],
        json!(format!("answer {}", SEEDED_TURNS - HEAD_LIMIT))
    );
    assert_eq!(
        head_body["messages"][59]["content"],
        json!(format!("answer {}", SEEDED_TURNS - 1))
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

async fn seed_large_session(store: &Store, session_id: SessionId, task_id: TaskId, turns: i64) {
    struct SeedRow {
        index: i64,
        event_seq: i64,
        event_id: String,
        message_id: String,
        run_id: String,
        turn_id: String,
        created_at: String,
        payload_json: String,
        input_json: String,
    }

    let session_id = session_id.0.to_string();
    let task_id = task_id.0.to_string();
    let started_at = chrono::Utc::now();
    // Seed fixture rows directly so this response-size test does not enqueue
    // projection work once per row before the explicit refresh below.
    let rows = (0..turns)
        .map(|index| {
            let created_at = started_at + chrono::Duration::milliseconds(index);
            SeedRow {
                index,
                event_seq: index + 1,
                event_id: uuid::Uuid::new_v4().to_string(),
                message_id: MessageId::new().0.to_string(),
                run_id: RunId::new().0.to_string(),
                turn_id: TurnId::new().0.to_string(),
                created_at: created_at.to_rfc3339(),
                payload_json: json!({
                    "kind": "large_head_checkpoint",
                    "turn_index": index,
                })
                .to_string(),
                input_json: json!({ "cmd": format!("echo {index}") }).to_string(),
            }
        })
        .collect::<Vec<_>>();

    let mut turn_builder = QueryBuilder::<Sqlite>::new(
        r#"INSERT INTO session_turns (
            turn_id, session_id, run_id, user_message_id, status, start_seq, end_seq,
            started_at, updated_at, assistant_partial, thought_partial, metrics_json,
            tool_total, tool_pending, tool_running, tool_completed, tool_failed
        ) "#,
    );
    turn_builder.push_values(&rows, |mut values, row| {
        values
            .push_bind(&row.turn_id)
            .push_bind(&session_id)
            .push_bind(&row.run_id)
            .push_bind(Option::<String>::None)
            .push_bind("completed")
            .push_bind(row.index + 1)
            .push_bind(row.index + 1)
            .push_bind(&row.created_at)
            .push_bind(&row.created_at)
            .push_bind(Option::<String>::None)
            .push_bind(Option::<String>::None)
            .push_bind(Option::<String>::None)
            .push_bind(1_i64)
            .push_bind(0_i64)
            .push_bind(0_i64)
            .push_bind(1_i64)
            .push_bind(0_i64);
    });
    turn_builder.build().execute(store.pool()).await.unwrap();

    let mut event_builder = QueryBuilder::<Sqlite>::new(
        r#"INSERT INTO session_events (
            seq, id, session_id, run_id, turn_id, event_type, payload_json, transient, created_at
        ) "#,
    );
    event_builder.push_values(&rows, |mut values, row| {
        values
            .push_bind(row.event_seq)
            .push_bind(&row.event_id)
            .push_bind(&session_id)
            .push_bind(&row.run_id)
            .push_bind(&row.turn_id)
            .push_bind("notice")
            .push_bind(&row.payload_json)
            .push_bind(0_i64)
            .push_bind(&row.created_at);
    });
    event_builder.build().execute(store.pool()).await.unwrap();

    let mut message_builder = QueryBuilder::<Sqlite>::new(
        r#"INSERT INTO messages (
            id, session_id, task_id, run_id, turn_id, turn_sequence, order_seq, role, content,
            attachments_json, delivery, delivered_at, created_at
        ) "#,
    );
    message_builder.push_values(&rows, |mut values, row| {
        values
            .push_bind(&row.message_id)
            .push_bind(&session_id)
            .push_bind(&task_id)
            .push_bind(&row.run_id)
            .push_bind(&row.turn_id)
            .push_bind(1_i64)
            .push_bind(Option::<i64>::None)
            .push_bind("assistant")
            .push_bind(format!("answer {}", row.index))
            .push_bind("[]")
            .push_bind("immediate")
            .push_bind(Option::<String>::None)
            .push_bind(&row.created_at);
    });
    message_builder.build().execute(store.pool()).await.unwrap();

    let mut tool_builder = QueryBuilder::<Sqlite>::new(
        r#"INSERT INTO session_turn_tools (
            session_id, tool_call_id, turn_id, tool_kind, provider_tool_name, title, subtitle,
            status, input_json, output_text, order_seq, first_event_seq, input_truncated,
            input_original_bytes, output_truncated, output_original_bytes, created_at, updated_at
        ) "#,
    );
    tool_builder.push_values(&rows, |mut values, row| {
        values
            .push_bind(&session_id)
            .push_bind(format!("tool-{}", row.index))
            .push_bind(&row.turn_id)
            .push_bind("execute")
            .push_bind("Bash")
            .push_bind("Bash")
            .push_bind(format!("turn {}", row.index))
            .push_bind("completed")
            .push_bind(&row.input_json)
            .push_bind(format!("output {}", row.index))
            .push_bind(1_i64)
            .push_bind(row.event_seq)
            .push_bind(0_i64)
            .push_bind(Option::<i64>::None)
            .push_bind(0_i64)
            .push_bind(Option::<i64>::None)
            .push_bind(&row.created_at)
            .push_bind(&row.created_at);
    });
    tool_builder.build().execute(store.pool()).await.unwrap();
}
