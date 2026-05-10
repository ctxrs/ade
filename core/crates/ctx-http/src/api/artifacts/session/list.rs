use super::*;

pub(in crate::api) async fn list_session_artifacts(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Artifact>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let mut artifacts = store
        .list_session_artifacts(session.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    for artifact in artifacts.iter_mut() {
        if !session_artifact_path_is_accessible(
            &state,
            &store,
            &session,
            StdPath::new(&artifact.absolute_path),
        )
        .await?
        {
            artifact.missing = Some(true);
        }
    }

    Ok(Json(artifacts))
}
