use ctx_core::ids::{RunId, TurnId};
use ctx_store::Store;
use sqlx::{QueryBuilder, Sqlite};

use super::{CtxUiSizedSeed, CtxUiTurnSeed};

pub(super) fn build_turns(seed: &CtxUiSizedSeed) -> Vec<CtxUiTurnSeed> {
    let started_at = chrono::Utc::now();
    let tool_turn_start = (seed.turn_count - 60).max(0);
    let tools_per_turn = seed.tool_count / 60;
    let tool_remainder = seed.tool_count % 60;

    (0..seed.turn_count)
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
        .collect()
}

pub(super) async fn insert_turns(store: &Store, session_id: &str, turns: &[CtxUiTurnSeed]) {
    let mut builder = QueryBuilder::<Sqlite>::new(
        r#"INSERT INTO session_turns (
            turn_id, session_id, run_id, user_message_id, status, start_seq, end_seq,
            started_at, updated_at, assistant_partial, thought_partial, metrics_json,
            tool_total, tool_pending, tool_running, tool_completed, tool_failed
        ) "#,
    );
    builder.push_values(turns, |mut values, row| {
        values
            .push_bind(&row.turn_id)
            .push_bind(session_id)
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
    builder.build().execute(store.pool()).await.unwrap();
}
