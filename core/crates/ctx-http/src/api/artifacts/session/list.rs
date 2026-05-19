use super::*;
use ctx_daemon::daemon::sessions::{SessionArtifactsRouteResponse, SessionRouteParams};
use ctx_daemon::daemon::SessionsHandle;

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
