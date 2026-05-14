use super::*;

pub(crate) async fn list_session_subagents(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
) -> Result<Json<Vec<SessionSummary>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    session_data_or_status(state.list_session_subagents_for_request(session_id).await).map(Json)
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionSubagentInvocationsQuery {
    turn_id: Option<String>,
}

pub(crate) async fn list_session_subagent_invocations(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    Query(q): Query<SessionSubagentInvocationsQuery>,
) -> Result<Json<Vec<SubagentInvocation>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let turn_id = match q.turn_id {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(TurnId(
                    uuid::Uuid::parse_str(trimmed).map_err(|_| StatusCode::BAD_REQUEST)?,
                ))
            }
        }
        None => None,
    };

    session_data_or_status(
        state
            .list_session_subagent_invocations_for_request(session_id, turn_id)
            .await,
    )
    .map(Json)
}

pub(crate) async fn get_session_subagent_invocation(
    State(state): State<SessionsHandle>,
    Path((session_id, id)): Path<(String, String)>,
) -> Result<Json<SubagentInvocation>, StatusCode> {
    let session_id =
        SessionId(uuid::Uuid::parse_str(&session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    session_data_or_status(
        state
            .get_session_subagent_invocation_for_request(session_id, &id)
            .await,
    )
    .map(Json)
}
