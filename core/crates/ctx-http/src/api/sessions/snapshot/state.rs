use super::*;

pub(crate) async fn get_session_state(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionState>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = store_for_existing_session_status_allow_archived(&state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if session.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let mut session_state = store
        .get_session_state(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let session = session.ok_or(StatusCode::NOT_FOUND)?;
    for artifact in session_state.artifacts.iter_mut() {
        if !super::super::super::artifacts::session_artifact_path_is_accessible(
            &state,
            &store,
            &session,
            std::path::Path::new(&artifact.absolute_path),
        )
        .await?
        {
            artifact.missing = Some(true);
        }
    }
    Ok(Json(session_state))
}
