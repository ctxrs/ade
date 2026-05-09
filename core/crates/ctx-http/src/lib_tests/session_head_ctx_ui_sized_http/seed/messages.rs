use ctx_core::ids::MessageId;
use ctx_store::Store;
use sqlx::{QueryBuilder, Sqlite};

use super::{CtxUiSizedSeed, CtxUiTurnSeed, BATCH};

pub(super) async fn seed_messages(
    store: &Store,
    session_id: &str,
    task_id: &str,
    seed: &CtxUiSizedSeed,
    turns: &[CtxUiTurnSeed],
) {
    let mut message_index = 0_i64;
    while message_index < seed.message_count {
        let end = (message_index + BATCH).min(seed.message_count);
        let rows = (message_index..end).collect::<Vec<_>>();
        let mut builder = QueryBuilder::<Sqlite>::new(
            r#"INSERT INTO messages (
                id, session_id, task_id, run_id, turn_id, turn_sequence, order_seq, role, content,
                attachments_json, delivery, delivered_at, created_at
            ) "#,
        );
        builder.push_values(&rows, |mut values, index| {
            let turn_index =
                (*index * seed.turn_count / seed.message_count).clamp(0, seed.turn_count - 1);
            let turn = &turns[turn_index as usize];
            values
                .push_bind(MessageId::new().0.to_string())
                .push_bind(session_id)
                .push_bind(task_id)
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
        builder.build().execute(store.pool()).await.unwrap();
        message_index = end;
    }
}
