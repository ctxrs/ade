use ctx_core::ids::{MessageId, RunId, SessionId, TaskId, TurnId};
use ctx_store::Store;
use sqlx::{QueryBuilder, Sqlite};

use super::*;

pub(super) struct CtxUiSizedSeed {
    pub(super) turn_count: i64,
    pub(super) message_count: i64,
    pub(super) tool_count: i64,
    pub(super) event_count: i64,
    pub(super) tool_output_bytes: usize,
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

pub(super) async fn seed_ctx_ui_sized_session(
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

pub(super) async fn latest_turn_id(store: &Store, session_id: SessionId) -> TurnId {
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

pub(super) async fn tail_turn_ids(store: &Store, session_id: SessionId, limit: i64) -> Vec<TurnId> {
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
