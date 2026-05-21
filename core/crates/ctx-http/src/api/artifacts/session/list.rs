use super::*;
use ctx_daemon::daemon::SessionsHandle;
use ctx_route_contracts::sessions::SessionRouteParams;
use ctx_session_artifacts::route_contract::SessionArtifactsRouteResponse;

pub(in crate::api) async fn list_session_artifacts(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
) -> Result<Json<SessionArtifactsRouteResponse>, StatusCode> {
    let artifacts = state
        .list_session_artifacts_with_missing_for_route_params(SessionRouteParams::new(id))
        .await
        .map_err(session_artifact_status)?;

    Ok(Json(artifacts))
}
