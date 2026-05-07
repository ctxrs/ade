use ctx_core::ids::{MessageId, RunId, SessionId, TaskId, TurnId};
use ctx_providers::adapters::ProviderAdapter;
use ctx_store::Store;
use serde::de::DeserializeOwned;
use sqlx::{QueryBuilder, Sqlite};
use std::time::{Duration, Instant};

use super::*;

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

async fn latest_turn_id(store: &Store, session_id: SessionId) -> TurnId {
    let value: String = sqlx::query_scalar(
        r#"SELECT turn_id
           FROM session_turns
           WHERE session_id = ?
           ORDER BY start_seq DESC
           LIMIT 1"#,
    )
    .bind(session_id.0.to_string())
    .fetch_one(store.pool())
    .await
    .unwrap();
    TurnId(uuid::Uuid::parse_str(&value).unwrap())
}

async fn tail_turn_ids(store: &Store, session_id: SessionId, limit: i64) -> Vec<TurnId> {
    let rows: Vec<String> = sqlx::query_scalar(
        r#"SELECT turn_id
           FROM session_turns
           WHERE session_id = ?
           ORDER BY start_seq DESC
           LIMIT ?"#,
    )
    .bind(session_id.0.to_string())
    .bind(limit)
    .fetch_all(store.pool())
    .await
    .unwrap();
    rows.into_iter()
        .map(|value| TurnId(uuid::Uuid::parse_str(&value).unwrap()))
        .collect()
}

struct CtxUiSizedSeed {
    turn_count: i64,
    message_count: i64,
    tool_count: i64,
    event_count: i64,
    tool_output_bytes: usize,
}

struct CtxUiTurnSeed {
    index: i64,
    run_id: String,
    turn_id: String,
    started_at: String,
    start_seq: i64,
    end_seq: Option<i64>,
    status: &'static str,
    tool_total: i64,
}

async fn seed_ctx_ui_sized_session(
    store: &Store,
    session_id: SessionId,
    task_id: TaskId,
    seed: CtxUiSizedSeed,
) {
    let session_id = session_id.0.to_string();
    let task_id = task_id.0.to_string();
    let started_at = chrono::Utc::now();
    let tool_turn_start = (seed.turn_count - 60).max(0);
    let tools_per_turn = seed.tool_count / 60;
    let tool_remainder = seed.tool_count % 60;
    let turns = (0..seed.turn_count)
        .map(|index| {
            let start_seq = 1 + (index * seed.event_count / seed.turn_count);
            let tool_total = if index < tool_turn_start {
                0
            } else {
                let offset = index - tool_turn_start;
                tools_per_turn + if offset < tool_remainder { 1 } else { 0 }
            };
            CtxUiTurnSeed {
                index,
                run_id: RunId::new().0.to_string(),
                turn_id: TurnId::new().0.to_string(),
                started_at: (started_at + chrono::Duration::milliseconds(index)).to_rfc3339(),
                start_seq,
                end_seq: if index + 1 == seed.turn_count {
                    None
                } else {
                    Some(start_seq + 1)
                },
                status: if index + 1 == seed.turn_count {
                    "running"
                } else {
                    "completed"
                },
                tool_total,
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
    turn_builder.push_values(&turns, |mut values, row| {
        values
            .push_bind(&row.turn_id)
            .push_bind(&session_id)
            .push_bind(&row.run_id)
            .push_bind(Option::<String>::None)
            .push_bind(row.status)
            .push_bind(row.start_seq)
            .push_bind(row.end_seq)
            .push_bind(&row.started_at)
            .push_bind(&row.started_at)
            .push_bind(Option::<String>::None)
            .push_bind(Option::<String>::None)
            .push_bind(Option::<String>::None)
            .push_bind(row.tool_total)
            .push_bind(0_i64)
            .push_bind(if row.status == "running" {
                1_i64
            } else {
                0_i64
            })
            .push_bind(if row.status == "running" {
                0_i64
            } else {
                row.tool_total
            })
            .push_bind(0_i64);
    });
    turn_builder.build().execute(store.pool()).await.unwrap();

    const BATCH: i64 = 500;
    let mut event_seq = 1_i64;
    while event_seq <= seed.event_count {
        let end = (event_seq + BATCH - 1).min(seed.event_count);
        let rows = (event_seq..=end).collect::<Vec<_>>();
        let mut event_builder = QueryBuilder::<Sqlite>::new(
            r#"INSERT INTO session_events (
                seq, id, session_id, run_id, turn_id, event_type, payload_json, transient, created_at
            ) "#,
        );
        event_builder.push_values(&rows, |mut values, seq| {
            let turn_index =
                ((*seq - 1) * seed.turn_count / seed.event_count).clamp(0, seed.turn_count - 1);
            let turn = &turns[turn_index as usize];
            let event_type = match *seq % 11 {
                0 => "tool_call",
                1 => "tool_result",
                2 => "assistant_message_inserted",
                3 => "assistant_complete",
                _ => "notice",
            };
            let payload = if turn.index + 1 == seed.turn_count
                && matches!(event_type, "tool_call" | "tool_result")
            {
                json!({
                    "kind": "ctx_ui_sized_fixture",
                    "seq": seq,
                    "turn_index": turn.index,
                    "tool_call_id": format!("ctx-ui-live-tool-{seq}"),
                    "order_seq": seed.tool_count + *seq,
                    "title": format!("Live fixture command {seq}"),
                    "status": if event_type == "tool_result" { "completed" } else { "pending" },
                    "rawInput": {
                        "cmd": "printf ctx-ui-live-fixture",
                        "seq": seq,
                    },
                    "output_text": format!("live fixture output {seq}"),
                })
            } else {
                json!({
                    "kind": "ctx_ui_sized_fixture",
                    "seq": seq,
                    "turn_index": turn.index,
                })
            };
            values
                .push_bind(*seq)
                .push_bind(uuid::Uuid::new_v4().to_string())
                .push_bind(&session_id)
                .push_bind(&turn.run_id)
                .push_bind(&turn.turn_id)
                .push_bind(event_type)
                .push_bind(payload.to_string())
                .push_bind(if *seq % 17 == 0 { 1_i64 } else { 0_i64 })
                .push_bind(&turn.started_at);
        });
        event_builder.build().execute(store.pool()).await.unwrap();
        event_seq = end + 1;
    }

    let mut message_index = 0_i64;
    while message_index < seed.message_count {
        let end = (message_index + BATCH).min(seed.message_count);
        let rows = (message_index..end).collect::<Vec<_>>();
        let mut message_builder = QueryBuilder::<Sqlite>::new(
            r#"INSERT INTO messages (
                id, session_id, task_id, run_id, turn_id, turn_sequence, order_seq, role, content,
                attachments_json, delivery, delivered_at, created_at
            ) "#,
        );
        message_builder.push_values(&rows, |mut values, index| {
            let turn_index =
                (*index * seed.turn_count / seed.message_count).clamp(0, seed.turn_count - 1);
            let turn = &turns[turn_index as usize];
            values
                .push_bind(MessageId::new().0.to_string())
                .push_bind(&session_id)
                .push_bind(&task_id)
                .push_bind(&turn.run_id)
                .push_bind(&turn.turn_id)
                .push_bind(*index)
                .push_bind(*index)
                .push_bind("assistant")
                .push_bind(format!("ctx-ui fixture answer {index}"))
                .push_bind("[]")
                .push_bind("immediate")
                .push_bind(Option::<String>::None)
                .push_bind(&turn.started_at);
        });
        message_builder.build().execute(store.pool()).await.unwrap();
        message_index = end;
    }

    let output_text = "x".repeat(seed.tool_output_bytes);
    let input_json = json!({
        "cmd": "printf ctx-ui-sized-fixture",
        "env": {"CTX_FIXTURE": "long-tail"},
        "payload": "y".repeat(256),
    })
    .to_string();
    let tool_turns = turns
        .iter()
        .filter(|turn| turn.tool_total > 0)
        .collect::<Vec<_>>();
    let mut tool_index = 0_i64;
    while tool_index < seed.tool_count {
        let end = (tool_index + BATCH).min(seed.tool_count);
        let rows = (tool_index..end).collect::<Vec<_>>();
        let mut tool_builder = QueryBuilder::<Sqlite>::new(
            r#"INSERT INTO session_turn_tools (
                session_id, tool_call_id, turn_id, tool_kind, provider_tool_name, title, subtitle,
                status, input_json, output_text, order_seq, first_event_seq, input_truncated,
                input_original_bytes, output_truncated, output_original_bytes, created_at, updated_at
            ) "#,
        );
        tool_builder.push_values(&rows, |mut values, index| {
            let mut turn = tool_turns[(*index as usize) % tool_turns.len()];
            if turn.index + 1 == seed.turn_count && tool_turns.len() > 1 {
                turn = tool_turns[tool_turns.len() - 2];
            }
            values
                .push_bind(&session_id)
                .push_bind(format!("ctx-ui-tool-{index}"))
                .push_bind(&turn.turn_id)
                .push_bind("exec")
                .push_bind("exec_command")
                .push_bind(format!("Fixture command {index}"))
                .push_bind(format!("turn {}", turn.index))
                .push_bind("completed")
                .push_bind(&input_json)
                .push_bind(&output_text)
                .push_bind(*index)
                .push_bind(turn.start_seq)
                .push_bind(0_i64)
                .push_bind(input_json.len() as i64)
                .push_bind(0_i64)
                .push_bind(output_text.len() as i64)
                .push_bind(&turn.started_at)
                .push_bind(&turn.started_at);
        });
        tool_builder.build().execute(store.pool()).await.unwrap();
        tool_index = end;
    }
}
