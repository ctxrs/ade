use super::*;

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionEventsQuery {
    pub(crate) after_seq: Option<i64>,
    pub(crate) limit: Option<u32>,
    pub(crate) tail: Option<u32>,
    pub(crate) include_transient: Option<String>,
}

pub(crate) async fn get_session_events(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    Query(q): Query<SessionEventsQuery>,
) -> Result<Json<ctx_core::models::SessionEventsPage>, StatusCode> {
    const DEFAULT_LIMIT: u32 = 200;
    const MAX_LIMIT: u32 = 1000;

    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let include_transient =
        super::parse_boolish_flag(q.include_transient.as_deref(), "include_transient")
            .map_err(|_| StatusCode::BAD_REQUEST)?;
    session_data_or_status(
        state
            .list_session_events_page(session_id, q.after_seq, limit, q.tail, include_transient)
            .await,
    )
    .map(Json)
}
