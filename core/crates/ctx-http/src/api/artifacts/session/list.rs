use super::*;
use ctx_daemon::daemon::SessionsHandle;

pub(in crate::api) async fn list_session_artifacts(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Artifact>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let artifacts = state
        .list_session_artifacts_with_missing_for_route(session_id)
        .await
        .map_err(session_artifact_status)?;

    Ok(Json(artifacts))
}
