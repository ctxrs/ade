use ctx_core::ids::{SessionId, TurnId};
use ctx_store::Store;

pub(in crate::lib_tests::session_head_ctx_ui_sized_http) async fn latest_turn_id(
    store: &Store,
    session_id: SessionId,
) -> TurnId {
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

pub(in crate::lib_tests::session_head_ctx_ui_sized_http) async fn tail_turn_ids(
    store: &Store,
    session_id: SessionId,
    limit: i64,
) -> Vec<TurnId> {
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
