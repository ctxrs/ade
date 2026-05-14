use super::*;

pub(crate) async fn get_session_state(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
) -> Result<Json<SessionState>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    session_data_or_status(state.load_session_state(session_id).await).map(Json)
}
