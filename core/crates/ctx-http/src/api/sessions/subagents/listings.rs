use super::*;

pub(crate) async fn list_session_subagents(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<SessionSummary>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = store_for_existing_session_status(&state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let subs = store
        .list_subagent_sessions(session.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(subs))
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionSubagentInvocationsQuery {
    turn_id: Option<String>,
}

pub(crate) async fn list_session_subagent_invocations(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionSubagentInvocationsQuery>,
) -> Result<Json<Vec<SubagentInvocation>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = store_for_existing_session_status(&state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

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

    let invocations = store
        .list_subagent_invocations_for_session(session.id, turn_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(invocations))
}

pub(crate) async fn get_session_subagent_invocation(
    State(state): State<Arc<AppState>>,
    Path((session_id, id)): Path<(String, String)>,
) -> Result<Json<SubagentInvocation>, StatusCode> {
    let session_id =
        SessionId(uuid::Uuid::parse_str(&session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = store_for_existing_session_status(&state, session_id).await?;
    let invocation = store
        .get_subagent_invocation(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if invocation.parent_session_id != session_id {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(invocation))
}
