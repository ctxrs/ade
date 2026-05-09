use ctx_store::Store;
use serde_json::json;
use sqlx::{QueryBuilder, Sqlite};

use super::{CtxUiSizedSeed, CtxUiTurnSeed, BATCH};

pub(super) async fn seed_events(
    store: &Store,
    session_id: &str,
    seed: &CtxUiSizedSeed,
    turns: &[CtxUiTurnSeed],
) {
    let mut event_seq = 1_i64;
    while event_seq <= seed.event_count {
        let end = (event_seq + BATCH - 1).min(seed.event_count);
        let rows = (event_seq..=end).collect::<Vec<_>>();
        let mut builder = QueryBuilder::<Sqlite>::new(
            r#"INSERT INTO session_events (
                seq, id, session_id, run_id, turn_id, event_type, payload_json, transient, created_at
            ) "#,
        );
        builder.push_values(&rows, |mut values, seq| {
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
            let payload = event_payload(seed, turn, *seq, event_type);
            values
                .push_bind(*seq)
                .push_bind(uuid::Uuid::new_v4().to_string())
                .push_bind(session_id)
                .push_bind(&turn.run_id)
                .push_bind(&turn.turn_id)
                .push_bind(event_type)
                .push_bind(payload.to_string())
                .push_bind(if *seq % 17 == 0 { 1_i64 } else { 0_i64 })
                .push_bind(&turn.started_at);
        });
        builder.build().execute(store.pool()).await.unwrap();
        event_seq = end + 1;
    }
}

fn event_payload(
    seed: &CtxUiSizedSeed,
    turn: &CtxUiTurnSeed,
    seq: i64,
    event_type: &str,
) -> serde_json::Value {
    if turn.index + 1 == seed.turn_count && matches!(event_type, "tool_call" | "tool_result") {
        return json!({
            "kind": "ctx_ui_sized_fixture",
            "seq": seq,
            "turn_index": turn.index,
            "tool_call_id": format!("ctx-ui-live-tool-{seq}"),
            "order_seq": seed.tool_count + seq,
            "title": format!("Live fixture command {seq}"),
            "status": if event_type == "tool_result" { "completed" } else { "pending" },
            "rawInput": {
                "cmd": "printf ctx-ui-live-fixture",
                "seq": seq,
            },
            "output_text": format!("live fixture output {seq}"),
        });
    }

    json!({
        "kind": "ctx_ui_sized_fixture",
        "seq": seq,
        "turn_index": turn.index,
    })
}
