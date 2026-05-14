use super::*;

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionHistoryQuery {
    pub(crate) before_seq: Option<i64>,
    pub(crate) limit: Option<u32>,
}

pub(crate) async fn get_session_history(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    Query(q): Query<SessionHistoryQuery>,
) -> Result<Json<SessionHistoryPage>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = q.limit.unwrap_or(60);
    session_data_or_status(
        state
            .load_session_history_page(session_id, q.before_seq, limit)
            .await,
    )
    .map(Json)
}

pub(crate) async fn list_session_turn_tools(
    State(state): State<SessionsHandle>,
    Path((id, turn_id)): Path<(String, String)>,
) -> Result<Json<Vec<SessionTurnTool>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let turn_id = TurnId(uuid::Uuid::parse_str(&turn_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    session_data_or_status(
        state
            .list_session_turn_tools_for_request(session_id, turn_id)
            .await,
    )
    .map(Json)
}
