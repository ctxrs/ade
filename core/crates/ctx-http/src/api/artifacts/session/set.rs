use super::*;
use ctx_daemon::daemon::sessions::{
    SessionArtifactRouteContext, SessionArtifactsRouteResponse, SessionRouteParams,
    SetSessionArtifactsRouteRequest,
};
use ctx_daemon::daemon::SessionsHandle;

pub(in crate::api) async fn set_session_artifacts(
    State(state): State<SessionsHandle>,
    mcp_auth: Option<Extension<ctx_mcp_auth::McpAuthContext>>,
    Path(id): Path<String>,
    Json(req): Json<SetSessionArtifactsRouteRequest>,
) -> Result<Json<SessionArtifactsRouteResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let artifacts = state
        .set_session_artifacts_for_route_params(
            SessionRouteParams::new(id),
            SessionArtifactRouteContext::new(mcp_auth.map(|Extension(auth)| auth)),
            req,
        )
        .await
        .map_err(session_artifact_api_error)?;

    Ok(Json(artifacts))
}
