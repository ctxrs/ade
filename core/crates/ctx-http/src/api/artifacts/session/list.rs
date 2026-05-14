use super::*;
use crate::daemon::SessionsHandle;

pub(in crate::api) async fn list_session_artifacts(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Artifact>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let (session, mut artifacts) = state
        .list_session_artifacts_for_route(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    for artifact in artifacts.iter_mut() {
        if !session_artifact_path_is_accessible(
            &state,
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
