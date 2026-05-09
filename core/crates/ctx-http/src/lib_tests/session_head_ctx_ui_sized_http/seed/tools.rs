use ctx_store::Store;
use serde_json::json;
use sqlx::{QueryBuilder, Sqlite};

use super::{CtxUiSizedSeed, CtxUiTurnSeed, BATCH};

pub(super) async fn seed_tools(
    store: &Store,
    session_id: &str,
    seed: &CtxUiSizedSeed,
    turns: &[CtxUiTurnSeed],
) {
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
        let mut builder = QueryBuilder::<Sqlite>::new(
            r#"INSERT INTO session_turn_tools (
                session_id, tool_call_id, turn_id, tool_kind, provider_tool_name, title, subtitle,
                status, input_json, output_text, order_seq, first_event_seq, input_truncated,
                input_original_bytes, output_truncated, output_original_bytes, created_at, updated_at
            ) "#,
        );
        builder.push_values(&rows, |mut values, index| {
            let mut turn = tool_turns[(*index as usize) % tool_turns.len()];
            if turn.index + 1 == seed.turn_count && tool_turns.len() > 1 {
                turn = tool_turns[tool_turns.len() - 2];
            }
            values
                .push_bind(session_id)
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
        builder.build().execute(store.pool()).await.unwrap();
        tool_index = end;
    }
}
