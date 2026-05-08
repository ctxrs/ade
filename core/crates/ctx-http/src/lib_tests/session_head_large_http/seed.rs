use ctx_core::ids::{MessageId, RunId, SessionId, TaskId, TurnId};
use ctx_store::Store;
use sqlx::{QueryBuilder, Sqlite};

pub(super) async fn seed_large_session(
    store: &Store,
    session_id: SessionId,
    task_id: TaskId,
    turns: i64,
) {
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
                payload_json: serde_json::json!({
                    "kind": "large_head_checkpoint",
                    "turn_index": index,
                })
                .to_string(),
                input_json: serde_json::json!({ "cmd": format!("echo {index}") }).to_string(),
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
